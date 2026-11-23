import { describe, expect, it, vi } from 'vitest'
import type { ActionContext, RainAction, SettingDef } from '@/actions/types'
import type { RainApi } from '@/data/api'
import type { SettingsPatch } from '@/data/types'
// actions.ts reaches the data layer through useResource, which would start a shell and a stream.
vi.mock('@/data', () => ({ api: {} }))

import { settingActions, settingDefaults } from './actions'
import { settingDefs } from './settingDefs'

function actionsOf(id: string): RainAction[] {
  const item: SettingDef | undefined = settingDefs.find((def) => def.id === id)
  if (!item) throw new Error(`no row ${id}`)
  // The provider reads only the target, never the context.
  return settingActions({ type: 'setting', id, item }, undefined as never)
}

/** A context whose api records the patches it is given and whose notice records the successes. */
function recordingContext() {
  const patches: SettingsPatch[] = []
  const successes: string[] = []
  const context = {
    api: { updateSettings: (patch: SettingsPatch) => (patches.push(patch), Promise.resolve()) } as unknown as RainApi,
    notice: { success: (text: string) => successes.push(text) },
  } as unknown as ActionContext
  return { context, patches, successes }
}

describe('Settings row actions: reset to default', () => {
  it('offers a reset only for the rows that have a default', () => {
    expect(actionsOf('hide-on-blur').map((action) => action.id)).toEqual(['change', 'reset'])
    expect(actionsOf('vault').map((action) => action.id)).toEqual(['change', 'reset'])
    expect(actionsOf('data-dir').map((action) => action.id)).toEqual(['change'])
    expect(actionsOf('game-folders').map((action) => action.id)).toEqual(['change'])
  })

  it('resets an Obsidian-following text to the empty text, which means "follow Obsidian again"', () => {
    expect(settingDefaults.vaultPath).toBe('')
    expect(settingDefaults.dailyNoteFolder).toBe('')
    expect(settingDefaults.dailyNoteFormat).toBe('')
  })

  it('sends exactly one field when a text row is reset', async () => {
    const { context, patches } = recordingContext()
    await actionsOf('note-format').find((action) => action.id === 'reset')?.run(context)
    expect(patches).toEqual([{ dailyNoteFormat: '' }])
  })
})

describe('Settings row actions: Claude monitoring', () => {
  it('offers a repair that turns the switch on again, whatever the state', async () => {
    const { context, patches, successes } = recordingContext()
    const repair = actionsOf('claude-monitor').find((action) => action.id === 'repair')
    expect(repair?.slot).toBe('secondary')

    await repair?.run(context)
    expect(patches).toEqual([{ monitorClaude: true }])
    expect(successes).toEqual(['Claude monitoring repaired'])
  })
})
