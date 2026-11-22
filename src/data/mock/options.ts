// The prototype options (scenario and simulated latency). They live in their own
// module so the engine and the store can read them without importing control.ts.
// Saved in localStorage so a refresh keeps the chosen scenario.

import { reactive } from 'vue'

export type Scenario = 'typical' | 'empty' | 'busy' | 'failing'
export interface PrototypeOptions { scenario: Scenario; latency: boolean }

const STORAGE_KEY = 'rain.prototype'
const SCENARIOS: readonly Scenario[] = ['typical', 'empty', 'busy', 'failing']
const DEFAULTS: PrototypeOptions = { scenario: 'typical', latency: true }

function load(): PrototypeOptions {
  try {
    const saved = JSON.parse(localStorage.getItem(STORAGE_KEY) ?? '{}') as Partial<PrototypeOptions>
    return {
      scenario: SCENARIOS.find((s) => s === saved.scenario) ?? DEFAULTS.scenario,
      latency: typeof saved.latency === 'boolean' ? saved.latency : DEFAULTS.latency,
    }
  } catch {
    // Storage blocked or the saved text is not JSON: start from the defaults.
    return { ...DEFAULTS }
  }
}

export const options = reactive<PrototypeOptions>(load())

export function saveOptions(): void {
  try {
    localStorage.setItem(STORAGE_KEY, JSON.stringify(options))
  } catch {
    // Storage blocked (private window): the options then last for this page only.
  }
}
