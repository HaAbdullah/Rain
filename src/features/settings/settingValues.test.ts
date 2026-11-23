import { describe, expect, it } from 'vitest'
import type { SettingDef } from '@/actions/types'
import type { Settings } from '@/data/types'
import { settingDefs } from './settingDefs'
import {
  isBooleanKey,
  joinPaths,
  patchFor,
  readLibraryPath,
  readValue,
  splitPaths,
  togglePatch,
} from './settingValues'

const SETTINGS: Settings = {
  hotkey: 'Ctrl+Backquote',
  hotkeyAvailable: true,
  hideOnBlur: true,
  resetAfterSeconds: 60,
  startWithWindows: true,
  dataDir: 'D:\\rain-data',
  vaultPath: 'D:\\notes',
  dailyNoteFormat: 'YYYY-MM-DD',
  dailyNoteFolder: 'Dailys',
  projectRoots: ['D:\\work', 'D:\\more'],
  gameFolders: ['D:\\games'],
  steamEnabled: true,
  steamPath: 'D:\\steam',
  epicEnabled: false,
  epicPath: '',
  calendarConnected: false,
  monitorClaude: true,
  monitorClaudeState: 'on',
  gameModeCountdownSeconds: 10,
}

function rowOf(id: string): SettingDef {
  const def = settingDefs.find((candidate) => candidate.id === id)
  if (!def) throw new Error(`no row ${id}`)
  return def
}

describe('readValue', () => {
  it.each([
    ['hotkey', 'Ctrl+Backquote'],
    ['hide-on-blur', true],
    ['reset-after', '60'],
    ['start-with-windows', true],
    ['data-dir', 'D:\\rain-data'],
    ['vault', 'D:\\notes'],
    ['note-folder', 'Dailys'],
    ['note-format', 'YYYY-MM-DD'],
    ['project-roots', 'D:\\work; D:\\more'],
    ['game-folders', 'D:\\games'],
    ['steam', true],
    ['epic', false],
    ['claude-monitor', true],
    ['countdown', '10'],
    ['calendar', 'Not connected'],
  ])('row %s shows %j', (id, expected) => {
    expect(readValue(rowOf(id), SETTINGS)).toBe(expected)
  })

  it('says Connected for a calendar that is, though the backend never reports one this round', () => {
    expect(readValue(rowOf('calendar'), { ...SETTINGS, calendarConnected: true })).toBe('Connected')
  })

  it('reads the library path beside the Steam and Epic switches', () => {
    expect(readLibraryPath(rowOf('steam'), SETTINGS)).toBe('D:\\steam')
    expect(readLibraryPath(rowOf('epic'), SETTINGS)).toBe('')
    expect(readLibraryPath(rowOf('vault'), SETTINGS)).toBeUndefined()
  })
})

describe('patchFor', () => {
  it('trims a text and sends an empty text as the request to clear the override', () => {
    expect(patchFor(rowOf('vault'), '  D:\\notes  ')).toEqual({ vaultPath: 'D:\\notes' })
    expect(patchFor(rowOf('note-folder'), '  Daily ')).toEqual({ dailyNoteFolder: 'Daily' })
    expect(patchFor(rowOf('note-format'), '')).toEqual({ dailyNoteFormat: '' })
    expect(patchFor(rowOf('data-dir'), ' E:\\data ')).toEqual({ dataDir: 'E:\\data' })
  })

  it('splits a folder list on semicolons and drops empty parts', () => {
    expect(patchFor(rowOf('project-roots'), ' D:\\a ;; D:\\b; ')).toEqual({ projectRoots: ['D:\\a', 'D:\\b'] })
    expect(patchFor(rowOf('game-folders'), 'D:\\games')).toEqual({ gameFolders: ['D:\\games'] })
    expect(patchFor(rowOf('game-folders'), '')).toEqual({ gameFolders: [] })
  })

  it('sends a select as the number the backend accepts, and nothing for another value', () => {
    expect(patchFor(rowOf('reset-after'), '300')).toEqual({ resetAfterSeconds: 300 })
    expect(patchFor(rowOf('reset-after'), '45')).toBeNull()
    expect(patchFor(rowOf('countdown'), '20')).toEqual({ gameModeCountdownSeconds: 20 })
    expect(patchFor(rowOf('countdown'), '15')).toBeNull()
  })

  it('sends a switch as one field, the Claude switch included', () => {
    expect(patchFor(rowOf('start-with-windows'), true)).toEqual({ startWithWindows: true })
    expect(patchFor(rowOf('claude-monitor'), false)).toEqual({ monitorClaude: false })
  })

  it('has no patch for a read-only row, so the page can never send a field the backend rejects', () => {
    expect(patchFor(rowOf('hotkey'), 'Ctrl+Space')).toBeNull()
    expect(patchFor(rowOf('calendar'), 'Connected')).toBeNull()
    expect(patchFor(rowOf('calendar'), true)).toBeNull()
  })
})

describe('togglePatch', () => {
  it('returns exactly one key for every switch', () => {
    for (const key of ['hideOnBlur', 'startWithWindows', 'steamEnabled', 'epicEnabled', 'monitorClaude'] as const) {
      expect(isBooleanKey(key)).toBe(true)
      expect(togglePatch(key, true)).toEqual({ [key]: true })
    }
  })

  it('does not treat a read-only field as a switch', () => {
    expect(isBooleanKey('calendarConnected')).toBe(false)
    expect(isBooleanKey('hotkeyAvailable')).toBe(false)
  })
})

describe('folder lists', () => {
  it('joins with a semicolon and a space, and splits back to the same list', () => {
    const paths = ['D:\\a', 'D:\\b']
    expect(joinPaths(paths)).toBe('D:\\a; D:\\b')
    expect(splitPaths(joinPaths(paths))).toEqual(paths)
  })
})

describe('the rows', () => {
  it('give every row a unique id', () => {
    const ids = settingDefs.map((def) => def.id)
    expect(new Set(ids).size).toBe(ids.length)
  })

  it('have a placeholder on each path row that may be empty', () => {
    for (const id of ['vault', 'project-roots', 'game-folders', 'steam', 'epic']) {
      expect(rowOf(id).placeholder, id).toBeTruthy()
    }
  })
})
