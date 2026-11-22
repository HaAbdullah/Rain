// Sample settings. Empty has no vault, project folders or calendar; Failing
// keeps the paths but the hotkey could not be registered.

import type { Settings } from '../../types'
import type { Scenario } from '../options'
import { DEFAULT_COUNTDOWN_SECONDS } from './gameMode'
import { VAULT_PATH } from './life'

const TYPICAL: Settings = {
  hotkey: 'Ctrl+Backquote',
  hotkeyAvailable: true,
  hideOnBlur: true,
  resetAfterSeconds: 60,
  startWithWindows: false,
  vaultPath: VAULT_PATH,
  dailyNoteFormat: 'YYYY-MM-DD',
  projectRoots: ['C:\\Dev'],
  steamEnabled: true,
  steamPath: 'C:\\Program Files (x86)\\Steam',
  epicEnabled: true,
  epicPath: 'C:\\Program Files\\Epic Games',
  calendarConnected: true,
  gameModeCountdownSeconds: DEFAULT_COUNTDOWN_SECONDS,
}

/**
 * Paths the sample machine does not have. Settings draws the found or missing glyph
 * from `mockControl.pathExists`; in Typical the Epic path is missing, like its failing source.
 */
export const MISSING_PATHS: ReadonlySet<string> = new Set([TYPICAL.epicPath])

const OVERRIDES: Record<Scenario, Partial<Settings>> = {
  typical: {},
  busy: {},
  empty: { vaultPath: '', projectRoots: [], calendarConnected: false },
  failing: { hotkeyAvailable: false },
}

export function seed(scenario: Scenario): Settings {
  return { ...TYPICAL, projectRoots: [...TYPICAL.projectRoots], ...OVERRIDES[scenario] }
}
