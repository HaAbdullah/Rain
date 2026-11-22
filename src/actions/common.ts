// Pieces that several features' actions share: pin to Home, copy to the
// clipboard, reveal in Explorer, and "hide after it worked".
import { refreshResources } from '@/composables/useResource'
import type { ID, PinnableType } from '@/data/types'
import type { ActionContext, RainAction } from './types'

/** The resource key that holds each pinnable type's list. */
const LIST_KEY: Record<PinnableType, string> = {
  app: 'apps',
  pool: 'pools',
  project: 'projects',
  game: 'games',
}

/** Ctrl+P: pin to Home, or unpin when it is pinned. */
export function pinAction(type: PinnableType, id: ID, pinned: boolean): RainAction {
  return {
    id: 'pin',
    label: pinned ? 'Unpin from Home' : 'Pin to Home',
    icon: pinned ? 'pin-off' : 'pin',
    slot: 'more',
    keys: 'Ctrl+KeyP',
    async run({ api, notice }) {
      await api.setPinned(type, id, !pinned)
      refreshResources('pinned', LIST_KEY[type])
      notice.success(pinned ? 'Unpinned' : 'Pinned to Home')
    },
  }
}

/** Copies `text` and says so. `what` names it in the label: "Copy path", "Copy cron". */
export function copyAction(what: string, text: string): RainAction {
  return {
    id: 'copy',
    label: `Copy ${what}`,
    icon: 'copy',
    slot: 'more',
    async run({ notice }) {
      await navigator.clipboard.writeText(text)
      notice.info(`Copied ${what}`)
    },
  }
}

/** Shows a file or folder in Explorer. */
export function revealAction(
  path: string,
  slot: RainAction['slot'],
  label = 'Reveal in Explorer',
): RainAction {
  return {
    id: 'reveal',
    label,
    icon: 'folder-search',
    slot,
    async run(ctx) {
      await ctx.api.revealPath(path)
      await hideAfterSuccess(ctx)
    },
  }
}

/**
 * Rain gets out of the way once it has handed over to another window
 * (a terminal, a note, Explorer), unless the user pinned it open with F10.
 */
export async function hideAfterSuccess(ctx: ActionContext): Promise<void> {
  if (!ctx.shell.keepOpen.value) await ctx.shell.hide()
}

/** F10, the header button and the "Keep open" command. */
export function toggleKeepOpen({ shell, notice }: ActionContext): void {
  shell.keepOpen.value = !shell.keepOpen.value
  notice.info(shell.keepOpen.value ? 'Keep open is on' : 'Keep open is off')
}
