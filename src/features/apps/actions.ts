// Actions for apps and pools (spec 4.3), and the hook that opens the pool editor.
import { ref } from 'vue'
import { copyAction, pinAction, revealAction } from '@/actions/common'
import type { ActionContext, ActionProvider, TargetOf } from '@/actions/types'
import { useLaunch } from '@/composables/useLaunch'
import { refreshResources } from '@/composables/useResource'
import type { Pool, PoolDraft } from '@/data/types'

/**
 * The pool editor's state. PoolEditorDialog (mounted by AppsScreen) is open
 * while `draft` is not null; it saves with api.savePool and sets it back to null.
 */
export const poolEditor = {
  draft: ref<PoolDraft | null>(null),

  /** Ctrl+N and the "New pool" command. */
  create() {
    poolEditor.draft.value = { name: '', members: [] }
  },

  edit(pool: Pool) {
    // A copy, so Cancel leaves the pool as it was.
    poolEditor.draft.value = {
      id: pool.id,
      name: pool.name,
      members: pool.members.map((member) => ({ ...member })),
    }
  },
}

/** The editor dialog only exists on the Apps screen, so go there first. */
function editPool(pool: Pool, ctx: ActionContext) {
  if (ctx.nav.displayed.value !== 'apps') ctx.nav.go('apps', undefined, { id: pool.id })
  poolEditor.edit(pool)
}

/** Enter must not start a second launch while one is running (the pane's buttons are disabled then). */
function isRunning(id: string): boolean {
  return useLaunch().runs.value[id]?.status === 'running'
}

export const appActions: ActionProvider<TargetOf<'app'>> = ({ item: app }) => [
  {
    id: 'launch',
    label: 'Launch',
    icon: 'rocket',
    slot: 'primary',
    run: async (ctx) => {
      if (isRunning(app.id)) return
      await ctx.launch({ type: 'app', id: app.id }, app.name)
    },
  },
  revealAction(app.exePath, 'secondary', 'Open file location'),
  pinAction('app', app.id, app.pinned),
  copyAction('path', app.exePath),
]

export const poolActions: ActionProvider<TargetOf<'pool'>> = ({ item: pool }) => [
  {
    id: 'launch',
    label: 'Launch all',
    icon: 'rocket',
    slot: 'primary',
    run: async (ctx) => {
      if (isRunning(pool.id)) return
      await ctx.launch({ type: 'pool', id: pool.id }, pool.name)
    },
  },
  {
    id: 'edit',
    label: 'Edit pool',
    icon: 'pencil',
    slot: 'secondary',
    run: (ctx) => editPool(pool, ctx),
  },
  pinAction('pool', pool.id, pool.pinned),
  {
    id: 'duplicate',
    label: 'Duplicate',
    icon: 'copy-plus',
    slot: 'more',
    async run({ api, notice }) {
      const copy = await api.savePool({ name: `${pool.name} copy`, members: pool.members })
      refreshResources('pools')
      notice.success(`Created ${copy.name}`)
    },
  },
  {
    id: 'delete',
    label: 'Delete pool',
    icon: 'trash-2',
    slot: 'more',
    danger: true,
    async run({ api, confirm, notice }) {
      const confirmed = await confirm({
        title: `Delete ${pool.name}?`,
        body: 'The pool is removed. Its apps stay installed and stay in the Apps list.',
        confirmLabel: 'Delete pool',
        danger: true,
      })
      if (!confirmed) return
      await api.deletePool(pool.id)
      refreshResources('pools', 'pinned', 'recent')
      notice.success(`Deleted ${pool.name}`)
    },
  },
]
