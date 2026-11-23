// Actions for a Settings row (spec 4.3): Enter changes it, Ctrl+Enter resets it.
// A switch that maps to a Settings field is toggled here through the api. Every
// other control (a Select to open, an Input to focus, a Button to press) lives
// in the screen, so Enter on those rows is handed to SettingsScreen.
import { shallowRef } from 'vue'
import type { ActionProvider, RainAction, SettingDef, TargetOf } from '@/actions/types'
import { refreshResources } from '@/composables/useResource'
import type { SettingsPatch } from '@/data/types'
import { isBooleanKey, togglePatch } from './settingValues'

/**
 * Set to a new object each time Enter asks the screen to operate a row's control.
 * SettingsScreen watches it: open the row's Select, focus its Input or press its Button.
 */
export const settingActivation = shallowRef<{ def: SettingDef } | null>(null)

/**
 * Defaults that hold on every machine (spec 7.9 and 8.3). A text that is empty here means
 * "follow Obsidian again". Folder lists and the data folder have no default of their own,
 * so those rows offer no "Reset to default".
 */
export const settingDefaults = {
  hideOnBlur: true,
  resetAfterSeconds: 60,
  startWithWindows: false,
  vaultPath: '',
  dailyNoteFormat: '',
  dailyNoteFolder: '',
  steamEnabled: true,
  epicEnabled: true,
  gameModeCountdownSeconds: 10,
} as const

type DefaultedKey = keyof typeof settingDefaults

function hasDefault(key: string | undefined): key is DefaultedKey {
  return key !== undefined && key in settingDefaults
}

/** The patch that puts one setting back to its default, a key per case so no cast is needed. */
function resetPatch(key: DefaultedKey): SettingsPatch {
  switch (key) {
    case 'hideOnBlur':
      return { hideOnBlur: settingDefaults.hideOnBlur }
    case 'resetAfterSeconds':
      return { resetAfterSeconds: settingDefaults.resetAfterSeconds }
    case 'startWithWindows':
      return { startWithWindows: settingDefaults.startWithWindows }
    case 'vaultPath':
      return { vaultPath: settingDefaults.vaultPath }
    case 'dailyNoteFormat':
      return { dailyNoteFormat: settingDefaults.dailyNoteFormat }
    case 'dailyNoteFolder':
      return { dailyNoteFolder: settingDefaults.dailyNoteFolder }
    case 'steamEnabled':
      return { steamEnabled: settingDefaults.steamEnabled }
    case 'epicEnabled':
      return { epicEnabled: settingDefaults.epicEnabled }
    case 'gameModeCountdownSeconds':
      return { gameModeCountdownSeconds: settingDefaults.gameModeCountdownSeconds }
  }
}

const CHANGE_LABEL: Record<SettingDef['kind'], string> = {
  switch: 'Toggle',
  select: 'Change',
  path: 'Edit',
  text: 'Edit',
  button: 'Run',
  info: 'Show hint',
}

function primaryFor(def: SettingDef): RainAction {
  const base = { id: 'change', label: CHANGE_LABEL[def.kind], icon: 'pencil', slot: 'primary' } as const
  const { key } = def

  if (def.kind === 'switch' && isBooleanKey(key)) {
    return {
      ...base,
      icon: 'toggle-right',
      async run({ api }) {
        const settings = await api.getSettings()
        await api.updateSettings(togglePatch(key, !settings[key]))
        refreshResources('settings')
      },
    }
  }

  if (def.kind === 'info') {
    // A read-only row has nothing to change; Enter shows its hint as a notice.
    return { ...base, icon: 'info', run: ({ notice }) => notice.info(def.hint ?? def.label) }
  }

  return {
    ...base,
    run() {
      settingActivation.value = { def }
    },
  }
}

export const settingActions: ActionProvider<TargetOf<'setting'>> = ({ item: def }) => {
  const actions = [primaryFor(def)]
  const { key } = def
  if (hasDefault(key)) {
    actions.push({
      id: 'reset',
      label: 'Reset to default',
      icon: 'rotate-ccw',
      slot: 'secondary',
      async run({ api, notice }) {
        await api.updateSettings(resetPatch(key))
        refreshResources('settings')
        notice.success(`${def.label}: back to the default`)
      },
    })
  }
  if (key === 'monitorClaude') {
    // Turning the switch on again installs whatever is missing and is safe to repeat, so it is offered in every state.
    actions.push({
      id: 'repair',
      label: 'Repair Claude hooks',
      icon: 'rotate-ccw',
      slot: 'secondary',
      async run({ api, notice }) {
        await api.updateSettings({ monitorClaude: true })
        refreshResources('settings')
        notice.success('Claude monitoring repaired')
      },
    })
  }
  return actions
}
