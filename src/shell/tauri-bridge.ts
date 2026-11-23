// The Bridge inside the Tauri window. A call is one `invoke('api_request')` that the
// Rust shell sends down the pipe; the live stream arrives as Tauri events that the
// shell's relay emits (src-tauri/src/bridge.rs and relay.rs, foundation 6.1 and 6.2).
// Like tauri.ts, this file may import @tauri-apps/api; nothing else may.
import { invoke } from '@tauri-apps/api/core'
import { listen, type UnlistenFn } from '@tauri-apps/api/event'
import type { BackendState, Bridge, StreamFrame } from './bridge'

/** Every frame of the backend's stream, as the shell received it. */
const EVENT_STREAM = 'rain:stream'
/** `{ state }`: the shell's view of the backend process. */
const EVENT_BACKEND = 'rain:backend'

/** The address under which the shell serves images through its custom scheme (foundation 6.3). */
const MEDIA_ORIGIN = 'http://rain-media.localhost'

/**
 * The one place that knows the address form of the custom scheme. On Windows Tauri 2 serves
 * `rain-media` as `http://rain-media.localhost` while `useHttpsScheme` is false; that window
 * setting must never change, because it would also move the page's saved selections.
 */
export function mediaUrl(ref: string): string {
  return `${MEDIA_ORIGIN}/${ref}`
}

/** What `GET /local/backend` answers: the shell's view of the backend process. */
interface BackendAnswer {
  state: BackendState
  startId?: string
}

/** Frames the relay sent before this page existed are gone, so a page that starts late asks for the state. */
async function askBackendState(onFrame: (frame: StreamFrame) => void, onState: (state: BackendState) => void) {
  try {
    const answer = await invoke<BackendAnswer>('api_request', { method: 'GET', path: '/local/backend' })
    onState(answer.state)
    if (answer.state !== 'up') return
    // The `hello` this page missed. The stream needs the start id to tell a later restart from the first run.
    if (answer.startId !== undefined) onFrame({ topic: 'hello', seq: 0, payload: { startId: answer.startId } })
    // Same effect as the `resync` the backend sends on every connect: refetch what is on screen.
    onFrame({ topic: 'resync', seq: 0, payload: { reason: 'connect' } })
  } catch {
    // The shell answers this path itself; if it cannot, the relay's own events still arrive.
  }
}

function openStream(handlers: { onFrame(frame: StreamFrame): void; onState(state: BackendState): void }): () => void {
  const listeners: Promise<UnlistenFn>[] = [
    listen<StreamFrame>(EVENT_STREAM, (event) => handlers.onFrame(event.payload)),
    listen<{ state: BackendState }>(EVENT_BACKEND, (event) => handlers.onState(event.payload.state)),
  ]
  // Ask only after both listeners are in place, so no event can fall between the two.
  void Promise.all(listeners).then(() => askBackendState(handlers.onFrame, handlers.onState))

  return () => {
    for (const listener of listeners) void listener.then((unlisten) => unlisten())
  }
}

export function createTauriBridge(): Bridge {
  return {
    // The shell rejects with an object that has the shape of a RainError, so nothing is caught here.
    // An empty answer (204) arrives as null; the Bridge promises undefined.
    request: async ({ method, path, body, timeoutMs }) =>
      (await invoke('api_request', { method, path, body, timeoutMs })) ?? undefined,
    openStream,
    mediaUrl,
  }
}
