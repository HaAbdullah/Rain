// A Bridge for tests: it records every request, answers with what the test queued, and lets
// the test push stream frames and state changes by hand.
import type { BackendState, Bridge, HttpMethod, StreamFrame } from '@/shell/bridge'

export interface RecordedCall {
  method: HttpMethod
  path: string
  body?: unknown
}

export interface FakeBridge extends Bridge {
  calls: RecordedCall[]
  /** The next request resolves with `value`. */
  answerWith(value: unknown): void
  /** The next request rejects with `reason`. */
  failWith(reason: unknown): void
  /** Delivers one frame to the page, as the real stream would. */
  push(frame: StreamFrame): void
  setState(state: BackendState): void
  /** How many times the page opened the stream. */
  streamsOpened: number
}

/** Media references get this prefix, so a test can see the rewrite without a real scheme. */
export const FAKE_MEDIA_ORIGIN = 'test://media-host/'

export function createFakeBridge(): FakeBridge {
  let outcome: () => Promise<unknown> = () => Promise.resolve(undefined)
  let handlers: Parameters<Bridge['openStream']>[0] | undefined

  const bridge: FakeBridge = {
    calls: [],
    streamsOpened: 0,

    request(req) {
      bridge.calls.push({ method: req.method, path: req.path, body: req.body })
      return outcome()
    },
    openStream(given) {
      handlers = given
      bridge.streamsOpened += 1
      return () => {}
    },
    mediaUrl: (ref) => FAKE_MEDIA_ORIGIN + ref,

    answerWith(value) {
      outcome = () => Promise.resolve(value)
    },
    failWith(reason) {
      outcome = () => Promise.reject(reason)
    },
    push(frame) {
      handlers?.onFrame(frame)
    },
    setState(state) {
      handlers?.onState(state)
    },
  }
  return bridge
}
