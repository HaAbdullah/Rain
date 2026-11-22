// Reads a row's value out of Settings (or the mock's options) and turns a changed
// value back into the patch for api.updateSettings. Each Settings field has its own
// case, so the types stay exact without casts.
import type { SettingDef } from '@/actions/types'
import { mockControl } from '@/data/mock/control'
import type { Settings } from '@/data/types'
import { APP_VERSION } from './settingDefs'

export type SettingValue = string | boolean

/** What the row's control shows. Prototype rows have no Settings field and read the mock. */
export function readValue(def: SettingDef, settings: Settings): SettingValue {
  switch (def.key) {
    case 'hotkey':
    case 'vaultPath':
    case 'dailyNoteFormat':
      return settings[def.key]
    case 'hideOnBlur':
    case 'startWithWindows':
    case 'steamEnabled':
    case 'epicEnabled':
    case 'calendarConnected':
      return settings[def.key]
    case 'resetAfterSeconds':
    case 'gameModeCountdownSeconds':
      return String(settings[def.key])
    case 'projectRoots':
      return settings.projectRoots.join('; ')
    default:
      if (def.id === 'mock-scenario') return mockControl.options.scenario
      if (def.id === 'mock-latency') return mockControl.options.latency
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
export function libraryPathPatch(def: SettingDef, path: string): Partial<Settings> | null {
  if (def.id === 'steam') return { steamPath: path }
  if (def.id === 'epic') return { epicPath: path }
  return null
}

/** The patch for a changed row, or null when the row or the value is not one that Settings stores. */
export function patchFor(def: SettingDef, value: SettingValue): Partial<Settings> | null {
  if (typeof value === 'boolean') {
    switch (def.key) {
      case 'hideOnBlur':
        return { hideOnBlur: value }
      case 'startWithWindows':
        return { startWithWindows: value }
      case 'steamEnabled':
        return { steamEnabled: value }
      case 'epicEnabled':
        return { epicEnabled: value }
      case 'calendarConnected':
        return { calendarConnected: value }
      default:
        return null
    }
  }

  switch (def.key) {
    case 'vaultPath':
      return { vaultPath: value.trim() }
    case 'dailyNoteFormat':
      return { dailyNoteFormat: value.trim() }
    case 'projectRoots':
      return {
        projectRoots: value
          .split(';')
          .map((root) => root.trim())
          .filter((root) => root !== ''),
      }
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

/** The label of a Button row. */
export function buttonLabel(def: SettingDef, value: SettingValue): string {
  if (def.key === 'calendarConnected') return value === true ? 'Disconnect' : 'Connect'
  return 'Reset'
}
