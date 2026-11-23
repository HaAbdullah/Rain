// The Bridge is the data layer's only door to the Rain backend: one request function,
// one live stream and one helper for image addresses. There are two implementations,
// the Tauri one (tauri-bridge.ts) and the plain-browser one (browser-bridge.ts); the
// Shell picks which. This file has no import from Tauri and none from the data layer.

export type HttpMethod = 'GET' | 'POST' | 'PUT' | 'PATCH' | 'DELETE'

/** The same shape as `RainError` in data/api.ts (the backend's error body). */
export interface BridgeError {
  code: string
  message: string
  source?: string
  reason?: string
}

/** One frame of the backend's live stream (foundation 5.1). */
export interface StreamFrame {
  topic: string
  seq: number
  payload: unknown
}

/** What the connection to the backend is doing (foundation 2.1). */
export type BackendState = 'starting' | 'up' | 'down' | 'stopped' | 'restarting'

export interface Bridge {
  /**
   * Sends one API call. `path` starts with "/" and omits the "/api/v1" prefix.
   * Resolves with the parsed JSON (undefined for 204). Rejects with a BridgeError.
   */
  request(req: { method: HttpMethod; path: string; body?: unknown; timeoutMs?: number }): Promise<unknown>
  /** Opens the one stream of the page. Returns a function that stops it. */
  openStream(handlers: { onFrame(frame: StreamFrame): void; onState(state: BackendState): void }): () => void
  /** Turns `media/app-icon/x?v=1` into an address an <img> can load. */
  mediaUrl(ref: string): string
}
