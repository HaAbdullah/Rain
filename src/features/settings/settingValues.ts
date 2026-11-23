// Reads a row's value out of Settings and turns a changed value back into the patch for
// api.updateSettings. Each Settings field has its own case, so the types stay exact without casts.
import type { SettingDef } from '@/actions/types'
import type { Settings, SettingsPatch } from '@/data/types'
import { APP_VERSION } from './settingDefs'

export type SettingValue = string | boolean

/** The switches that Settings stores and the user may change. */
export type BooleanSettingKey = 'hideOnBlur' | 'startWithWindows' | 'steamEnabled' | 'epicEnabled' | 'monitorClaude'

export function isBooleanKey(key: keyof Settings | undefined): key is BooleanSettingKey {
  return (
    key === 'hideOnBlur' ||
    key === 'startWithWindows' ||
    key === 'steamEnabled' ||
    key === 'epicEnabled' ||
    key === 'monitorClaude'
  )
}

/** A field that holds several folders is one text field with the folders separated by semicolons. */
export const splitPaths = (text: string): string[] =>
  text
    .split(';')
    .map((part) => part.trim())
    .filter((part) => part !== '')
export const joinPaths = (paths: string[]): string => paths.join('; ')

/** What the row's control shows. The About row has no Settings field and shows the app version. */
export function readValue(def: SettingDef, settings: Settings): SettingValue {
  switch (def.key) {
    case 'hotkey':
    case 'dataDir':
    case 'vaultPath':
    case 'dailyNoteFormat':
    case 'dailyNoteFolder':
      return settings[def.key]
    case 'hideOnBlur':
    case 'startWithWindows':
    case 'steamEnabled':
    case 'epicEnabled':
    case 'monitorClaude':
      return settings[def.key]
    case 'calendarConnected':
      // Read-only fact, shown as text on an info row.
      return settings.calendarConnected ? 'Connected' : 'Not connected'
    case 'resetAfterSeconds':
    case 'gameModeCountdownSeconds':
      return String(settings[def.key])
    case 'projectRoots':
    case 'gameFolders':
      return joinPaths(settings[def.key])
    default:
      return def.id === 'about' ? `Version ${APP_VERSION}` : ''
  }
}

/** The library path that belongs to a Steam or Epic row. */
export function readLibraryPath(def: SettingDef, settings: Settings): string | undefined {
  if (def.id === 'steam') return settings.steamPath
  if (def.id === 'epic') return settings.epicPath
  return undefined
}

/** The path field next to a Steam or Epic switch. */
export function libraryPathPatch(def: SettingDef, path: string): SettingsPatch | null {
  if (def.id === 'steam') return { steamPath: path }
  if (def.id === 'epic') return { epicPath: path }
  return null
}

/** The patch that sets one switch. A key per case, so the patch type stays exact without casts. */
export function togglePatch(key: BooleanSettingKey, value: boolean): SettingsPatch {
  switch (key) {
    case 'hideOnBlur':
      return { hideOnBlur: value }
    case 'startWithWindows':
      return { startWithWindows: value }
    case 'steamEnabled':
      return { steamEnabled: value }
    case 'epicEnabled':
      return { epicEnabled: value }
    case 'monitorClaude':
      return { monitorClaude: value }
  }
}

/** The patch for a changed row, or null when the row or the value is not one that Settings stores. */
export function patchFor(def: SettingDef, value: SettingValue): SettingsPatch | null {
  if (typeof value === 'boolean') return isBooleanKey(def.key) ? togglePatch(def.key, value) : null

  switch (def.key) {
    // An empty text clears the override: the backend then follows Obsidian again (or, for the data folder, uses its default).
    case 'dataDir':
      return { dataDir: value.trim() }
    case 'vaultPath':
      return { vaultPath: value.trim() }
    case 'dailyNoteFormat':
      return { dailyNoteFormat: value.trim() }
    case 'dailyNoteFolder':
      return { dailyNoteFolder: value.trim() }
    case 'projectRoots':
      return { projectRoots: splitPaths(value) }
    case 'gameFolders':
      return { gameFolders: splitPaths(value) }
    case 'resetAfterSeconds':
      if (value === '30') return { resetAfterSeconds: 30 }
      if (value === '60') return { resetAfterSeconds: 60 }
      return value === '300' ? { resetAfterSeconds: 300 } : null
    case 'gameModeCountdownSeconds':
      if (value === '5') return { gameModeCountdownSeconds: 5 }
      if (value === '10') return { gameModeCountdownSeconds: 10 }
      if (value === '20') return { gameModeCountdownSeconds: 20 }
      return value === '30' ? { gameModeCountdownSeconds: 30 } : null
    default:
      return null
  }
}
