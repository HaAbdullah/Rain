// Per-screen UI state that must outlive the screen component (only the active
// screen is mounted): a selected id, an open folder, the month on show.
// It is forgotten when Rain resets to Home after a long hide (spec 2.2).
import { reactive } from 'vue'

interface Entry {
  state: Record<string, unknown>
  initial: Record<string, unknown>
}

const entries = new Map<string, Entry>()

/**
 * Returns the reactive state stored under `key`, creating it from `initial` on first use.
 * `initial` must be plain data (it is copied with structuredClone).
 */
export function useViewState<T extends object>(key: string, initial: T): T {
  let entry = entries.get(key)
  if (!entry) {
    entry = {
      state: reactive(structuredClone(initial) as Record<string, unknown>),
      initial: structuredClone(initial) as Record<string, unknown>,
    }
    entries.set(key, entry)
  }
  return entry.state as T
}

/**
 * Puts every stored state back to its initial value. The objects are reset in
 * place, so a screen that is mounted right now sees the reset as well.
 */
export function resetViewState(): void {
  for (const { state, initial } of entries.values()) {
    for (const key of Object.keys(state)) {
      if (!(key in initial)) delete state[key]
    }
    Object.assign(state, structuredClone(initial))
  }
}
