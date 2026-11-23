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
  {
    id: 'data-dir',
    group: 'General',
    label: 'Data folder',
    hint: 'Your pools, jobs and habits. Rain restarts its service when this changes',
    kind: 'path',
    key: 'dataDir',
  },

  {
    id: 'vault',
    group: 'Sources',
    label: 'Obsidian vault',
    hint: 'Follows Obsidian when it has one vault. Rain reads your daily notes and writes only the habit you tick',
    kind: 'path',
    key: 'vaultPath',
    placeholder: 'Not set',
  },
  {
    id: 'note-folder',
    group: 'Sources',
    label: 'Daily notes folder',
    hint: 'Inside the vault. Follows Obsidian until you change it',
    kind: 'text',
    key: 'dailyNoteFolder',
  },
  {
    id: 'note-format',
    group: 'Sources',
    label: 'Daily note format',
    hint: 'Follows Obsidian until you change it',
    kind: 'text',
    key: 'dailyNoteFormat',
  },
  {
    id: 'project-roots',
    group: 'Sources',
    label: 'Project folders',
    hint: 'Separate several folders with a semicolon',
    kind: 'path',
    key: 'projectRoots',
    placeholder: 'Not set',
  },
  {
    id: 'game-folders',
    group: 'Sources',
    label: 'Game folders',
    hint: 'Every folder inside is one game. Separate several with a semicolon',
    kind: 'path',
    key: 'gameFolders',
    placeholder: 'Not set',
  },
  { id: 'steam', group: 'Sources', label: 'Steam library', kind: 'switch', key: 'steamEnabled', placeholder: 'Not found' },
  { id: 'epic', group: 'Sources', label: 'Epic library', kind: 'switch', key: 'epicEnabled', placeholder: 'Not found' },
  { id: 'calendar', group: 'Sources', label: 'Calendar', hint: 'Coming later', kind: 'info', key: 'calendarConnected' },
  {
    id: 'claude-monitor',
    group: 'Sources',
    label: 'Monitor Claude sessions',
    hint: 'Lets Rain show your sessions and answer their permission requests',
    kind: 'switch',
    key: 'monitorClaude',
  },

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
