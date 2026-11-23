// The Bridge for a plain browser tab (`npm run dev`): fetch and EventSource, both
// through the Vite dev server, which forwards them to the backend's pipe
// (vite/pipe-proxy.ts). Failures become the same error shape the backend sends
// (foundation 4.5 and 6.6), so the data layer never sees a raw fetch error.
import { TOPIC_NAMES } from '@/bindings/rain-types'
import type { BackendState, Bridge, BridgeError, StreamFrame } from './bridge'

const API_BASE = '/api/v1'
const DEFAULT_TIMEOUT_MS = 20_000

/** A call with a body or a side effect must carry this header; the proxy refuses it otherwise. */
const DEV_HEADER = 'X-Rain-Dev'

/** Frames have a named `event:` line, and an EventSource only hears the names it listens for. */
const STREAM_EVENT_NAMES = ['hello', 'resync', 'lifecycle', ...TOPIC_NAMES]

/** Waits before reconnecting a stream that the browser gave up on (foundation 5.5). */
const RECONNECT_DELAYS_MS = [500, 1000, 2000]
const RECONNECT_REPEAT_MS = 5000

export function reconnectDelayMs(failedAttempts: number): number {
  return RECONNECT_DELAYS_MS[failedAttempts] ?? RECONNECT_REPEAT_MS
}

function unavailable(message: string, reason: string): BridgeError {
  return { code: 'unavailable', message, source: 'backend', reason }
}

/** An error body the backend wrote, or undefined when the body is something else. */
function readErrorBody(body: unknown): BridgeError | undefined {
  if (typeof body !== 'object' || body === null) return undefined
  const { code, message } = body as Partial<BridgeError>
  return typeof code === 'string' && typeof message === 'string' ? (body as BridgeError) : undefined
}

/** The class of a failure that came without a proper error body (foundation 4.5). */
function codeForStatus(status: number): string {
  if (status === 404) return 'notFound'
  return status === 502 || status === 503 || status === 504 ? 'unavailable' : 'failed'
}

function badResponse(code: string, message: string): BridgeError {
  return { code, message, source: 'backend', reason: 'badResponse' }
}

function transportError(reason: unknown): BridgeError {
  if (reason instanceof DOMException && reason.name === 'TimeoutError') {
    return unavailable('Rain backend did not answer in time', 'timeout')
  }
  return unavailable('Could not reach the Rain backend', 'transport')
}

async function request(req: Parameters<Bridge['request']>[0]): Promise<unknown> {
  const headers: Record<string, string> = {}
  if (req.method !== 'GET') headers[DEV_HEADER] = '1'
  if (req.body !== undefined) headers['Content-Type'] = 'application/json'

  let response: Response
  let text: string
  try {
    response = await fetch(API_BASE + req.path, {
      method: req.method,
      headers,
      body: req.body === undefined ? undefined : JSON.stringify(req.body),
      signal: AbortSignal.timeout(req.timeoutMs ?? DEFAULT_TIMEOUT_MS),
    })
    text = await response.text()
  } catch (reason) {
    throw transportError(reason)
  }

  if (response.status === 204) return undefined

  let body: unknown
  try {
    body = JSON.parse(text)
  } catch {
    const code = response.ok ? 'failed' : codeForStatus(response.status)
    throw badResponse(code, 'Rain backend sent an answer that could not be read')
  }
  if (response.ok) return body
  throw readErrorBody(body) ?? badResponse(codeForStatus(response.status), 'Rain backend sent an unexpected answer')
}

/** A frame that is not JSON or has no topic and sequence number is a backend bug; the next frame is still good, so it is dropped. */
export function parseFrame(data: string): StreamFrame | undefined {
  let parsed: unknown
  try {
    parsed = JSON.parse(data)
  } catch {
    return undefined
  }
  if (typeof parsed !== 'object' || parsed === null) return undefined
  const { topic, seq } = parsed as Partial<StreamFrame>
  return typeof topic === 'string' && typeof seq === 'number' ? (parsed as StreamFrame) : undefined
}

/**
 * One EventSource for the page. The browser reconnects by itself after a dropped
 * connection, but gives up for good after a non-200 answer (the proxy's 503 while
 * the backend is down), so a closed source is recreated here after a backoff.
 */
function openStream(handlers: { onFrame(frame: StreamFrame): void; onState(state: BackendState): void }): () => void {
  let source: EventSource | null = null
  let retryTimer: number | undefined
  let failedAttempts = 0

  function connect() {
    const current = new EventSource(`${API_BASE}/stream`)
    source = current

    current.onopen = () => {
      failedAttempts = 0
      handlers.onState('up')
    }
    current.onerror = () => {
      handlers.onState('down')
      if (current.readyState !== EventSource.CLOSED) return // the browser is retrying by itself
      current.close()
      retryTimer = window.setTimeout(connect, reconnectDelayMs(failedAttempts))
      failedAttempts += 1
    }
    // The `data` line holds the whole frame, so the event name is not needed.
    for (const name of STREAM_EVENT_NAMES) {
      current.addEventListener(name, (event) => {
        const frame = parseFrame((event as MessageEvent<string>).data)
        if (frame) handlers.onFrame(frame)
      })
    }
  }

  connect()
  return () => {
    window.clearTimeout(retryTimer)
    source?.close()
  }
}

/** Images come through the same dev proxy as the calls: `media/app-icon/x?v=1` is `/api/v1/media/app-icon/x?v=1`. */
export function mediaUrl(ref: string): string {
  return `${API_BASE}/${ref}`
}

export function createBrowserBridge(): Bridge {
  return {
    request,
    openStream,
    mediaUrl,
  }
}
