// Prototype-only controls. They are not part of RainApi and disappear with the
// mock: the sample-data scenario, simulated latency, pausing the mock's timers
// while the window is hidden, and a permission request on demand.

import { readonly } from 'vue'
import { setPaused } from './engine'
import { options, saveOptions } from './options'
import type { PrototypeOptions, Scenario } from './options'
import { MISSING_PATHS } from './seed/settings'
import { markWorkingSessionsActive, requestPermission } from './slices/sessions'
import { resetState, state } from './store'

export type { PrototypeOptions, Scenario }

export interface MockControl {
  /** Reactive, and saved in localStorage('rain.prototype'). */
  options: Readonly<PrototypeOptions>
  /** Switches the sample data and reseeds. */
  setScenario(scenario: Scenario): void
  setLatency(on: boolean): void
  /** Moves one working session to "waiting on you". */
  simulatePermissionRequest(): void
  /** Throws away every change and reseeds the current scenario. */
  reset(): void
  /** Freezes every mock timer. AppShell calls this with shell.hidden. */
  setPaused(paused: boolean): void
  /** Whether a path from Settings exists on the sample machine. An empty path never does. */
  pathExists(path: string): boolean
  /** Runs `cb` after every reseed (setScenario and reset). Returns a function that removes it. */
  onReseed(cb: () => void): () => void
}

const reseedListeners = new Set<() => void>()

function reseed(): void {
  resetState()
  for (const cb of [...reseedListeners]) cb()
}

export const mockControl: MockControl = {
  options: readonly(options),

  setScenario(scenario) {
    options.scenario = scenario
    saveOptions()
    reseed()
  },

  setLatency(on) {
    options.latency = on
    saveOptions()
  },

  simulatePermissionRequest: requestPermission,
  reset: reseed,
  setPaused(paused) {
    setPaused(paused)
    if (!paused) markWorkingSessionsActive()
  },

  pathExists(path) {
    if (path === '' || MISSING_PATHS.has(path)) return false
    // Failing has a vault path in Settings but no vault there: Life says "not found", so Settings must too.
    return !(state.vaultPath === undefined && path === state.settings.vaultPath)
  },

  onReseed(cb) {
    reseedListeners.add(cb)
    return () => {
      reseedListeners.delete(cb)
    }
  },
}
