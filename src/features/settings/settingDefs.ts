// The rows of the Settings screen (spec 7.9), in the order they are drawn, and the
// fixed choices of the Select rows.
import type { SettingDef } from '@/actions/types'

export const settingDefs: SettingDef[] = [
  {
    id: 'hotkey',
    group: 'General',
    label: 'Summon hotkey',
    hint: 'Takes this chord from other apps while Rain runs',
    kind: 'info',
    key: 'hotkey',
  },
  { id: 'hide-on-blur', group: 'General', label: 'Hide when focus is lost', kind: 'switch', key: 'hideOnBlur' },
  { id: 'reset-after', group: 'General', label: 'Return to Home after', kind: 'select', key: 'resetAfterSeconds' },
  { id: 'start-with-windows', group: 'General', label: 'Start with Windows', kind: 'switch', key: 'startWithWindows' },

  { id: 'vault', group: 'Sources', label: 'Obsidian vault', kind: 'path', key: 'vaultPath' },
  { id: 'note-format', group: 'Sources', label: 'Daily note format', kind: 'text', key: 'dailyNoteFormat' },
  {
    id: 'project-roots',
    group: 'Sources',
    label: 'Project folders',
    hint: 'Separate several folders with a semicolon',
    kind: 'path',
    key: 'projectRoots',
  },
  { id: 'steam', group: 'Sources', label: 'Steam library', kind: 'switch', key: 'steamEnabled' },
  { id: 'epic', group: 'Sources', label: 'Epic library', kind: 'switch', key: 'epicEnabled' },
  { id: 'calendar', group: 'Sources', label: 'Google Calendar', kind: 'button', key: 'calendarConnected' },

  { id: 'countdown', group: 'Game mode', label: 'Countdown', kind: 'select', key: 'gameModeCountdownSeconds' },

  { id: 'mock-scenario', group: 'Prototype', label: 'Mock scenario', kind: 'select' },
  { id: 'mock-latency', group: 'Prototype', label: 'Simulated latency', kind: 'switch' },
  { id: 'mock-reset', group: 'Prototype', label: 'Reset mock data', kind: 'button' },

  { id: 'about', group: 'About', label: 'Rain', hint: 'A personal launcher for this PC', kind: 'info' },
]

export interface SelectChoice {
  value: string
  label: string
}

/** The choices of each Select row, by row id. */
export const selectChoices: Record<string, SelectChoice[]> = {
  'reset-after': [
    { value: '30', label: '30 s' },
    { value: '60', label: '60 s' },
    { value: '300', label: '5 min' },
  ],
  countdown: [
    { value: '5', label: '5 s' },
    { value: '10', label: '10 s' },
    { value: '20', label: '20 s' },
    { value: '30', label: '30 s' },
  ],
  'mock-scenario': [
    { value: 'typical', label: 'Typical' },
    { value: 'empty', label: 'Empty' },
    { value: 'busy', label: 'Busy' },
    { value: 'failing', label: 'Failing' },
  ],
}

export const APP_VERSION = '0.1.0'
