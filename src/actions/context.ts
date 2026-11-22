// Builds the ActionContext and runs actions with it. Every caller (the keymap,
// the Ctrl+K panel, a row click) goes through `runAction`, so an
// action that fails always ends as a notice and never as an unhandled rejection.
import { useConfirm } from '@/composables/useConfirm'
import { useLaunch } from '@/composables/useLaunch'
import { useLogDialog } from '@/composables/useLogDialog'
import { useNotice } from '@/composables/useNotice'
import { api } from '@/data'
import { errorMessage } from '@/lib/errors'
import { useNav } from '@/nav/useNav'
import { shell } from '@/shell'
import { actionsFor } from './registry'
import type { ActionContext, ActionTarget, RainAction } from './types'

/** A fresh context. `where` is read now, so build one per use instead of keeping it. */
export function actionContext(): ActionContext {
  const nav = useNav()
  return {
    where: nav.displayed.value === 'home' ? 'home' : 'section',
    api,
    nav,
    shell,
    notice: useNotice(),
    confirm: useConfirm(),
    launch: useLaunch().launch,
    openLog: useLogDialog().open,
  }
}

/** Runs one action. A rejection (a RainError from the api) becomes an error notice. */
export async function runAction(action: RainAction, ctx: ActionContext = actionContext()): Promise<void> {
  try {
    await action.run(ctx)
  } catch (reason) {
    ctx.notice.error(errorMessage(reason, `${action.label} failed`))
  }
}

/** Runs what Enter would run on the target: a row click, a double-click, an Alt digit. */
export function runPrimary(target: ActionTarget): void {
  const ctx = actionContext()
  const primary = actionsFor(target, ctx).find((action) => action.slot === 'primary')
  if (primary) void runAction(primary, ctx)
}
