// Actions for a Settings row (spec 4.3): Enter changes it, Ctrl+Enter resets it.
// A switch that maps to a Settings field is toggled here through the api. Every
// other control (a Select to open, an Input to focus, a Button to press) lives
// in the screen, so Enter on those rows is handed to SettingsScreen.
import { shallowRef } from 'vue'
import type { ActionProvider, RainAction, SettingDef, TargetOf } from '@/actions/types'
import { refreshResources } from '@/composables/useResource'
import type { Settings } from '@/data/types'

/**
 * Set to a new object each time Enter asks the screen to operate a row's control.
 * SettingsScreen watches it: open the row's Select, focus its Input or press its Button.
 */
export const settingActivation = shallowRef<{ def: SettingDef } | null>(null)

/**
 * Defaults that hold on every machine (spec 7.9 and 8.3). Paths have none,
 * so those rows offer no "Reset to default".
 */
export const settingDefaults: Partial<Settings> & { resetAfterSeconds: number } = {
  hideOnBlur: true,
  resetAfterSeconds: 60,
  startWithWindows: false,
  dailyNoteFormat: 'YYYY-MM-DD',
  steamEnabled: true,
  epicEnabled: true,
  gameModeCountdownSeconds: 10,
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

  if (def.kind === 'switch' && key) {
    return {
      ...base,
      icon: 'toggle-right',
      async run({ api }) {
        const settings = await api.getSettings()
        await api.updateSettings({ [key]: !settings[key] })
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
  if (key && key in settingDefaults) {
    actions.push({
      id: 'reset',
      label: 'Reset to default',
      icon: 'rotate-ccw',
      slot: 'secondary',
      async run({ api, notice }) {
        await api.updateSettings({ [key]: settingDefaults[key] })
        refreshResources('settings')
        notice.success(`${def.label}: back to the default`)
      },
    })
  }
  return actions
}
