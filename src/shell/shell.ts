// The Shell is the app's only door to its window: show, hide, maximise, zoom and
// "why may the window not hide right now". There are two implementations,
// TauriShell (the real window) and BrowserShell (a simulated hide for `npm run dev`).
// It also carries the Bridge, because which window we are in decides how the backend is reached.
import type { Ref } from 'vue'
import type { Bridge } from './bridge'

export interface Size {
  width: number
  height: number
}

/** The layout needs at least this much room, in CSS pixels. Zoom makes the window's minimum size grow with it. */
export const MIN_LAYOUT: Size = { width: 1040, height: 680 }

export interface Shell {
  readonly kind: 'tauri' | 'browser'
  /** How the data layer reaches the backend from this kind of window. */
  readonly bridge: Bridge
  readonly hidden: Readonly<Ref<boolean>>
  /** F10: while true the window stays open on blur and after a launch. */
  readonly keepOpen: Ref<boolean>
  show(): Promise<void>
  hide(): Promise<void>
  /** F11: true while the window fills the work area (the taskbar stays visible). Follows the real window. */
  readonly maximized: Readonly<Ref<boolean>>
  toggleMaximize(): Promise<void>
  /** The room the page may use on this monitor, in logical pixels. Null when it is not known. */
  workArea(): Promise<Size | null>
  /**
   * Sets the zoom of the page (percent), raises the window's minimum size to
   * MIN_LAYOUT times the zoom and grows the window when it is smaller than that.
   */
  applyZoom(percent: number): Promise<void>
  /** Suspends hide on blur until the returned function is called. */
  holdOpen(reason: string): () => void
  /** Runs after the window was shown and focused (hotkey, second launch, show()). */
  onSummon(cb: () => void): () => void
  /** Runs after the window was hidden, whoever hid it. */
  onHide(cb: () => void): () => void
}

/** A set of callbacks with the "returns its own unsubscribe" shape the Shell uses. */
export interface Signal {
  on(cb: () => void): () => void
  emit(): void
}

export function createSignal(): Signal {
  const listeners = new Set<() => void>()
  return {
    on(cb) {
      listeners.add(cb)
      return () => listeners.delete(cb)
    },
    emit() {
      // Copy first: a listener may unsubscribe itself while it runs.
      for (const cb of [...listeners]) cb()
    },
  }
}
