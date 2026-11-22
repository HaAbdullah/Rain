// The one lookup from "what is selected" to "what can be done with it" (spec 6.4).
// Each feature folder owns the providers for its item types; this file only
// routes by type and is not edited after the foundation phase.
import { appActions, poolActions } from '@/features/apps/actions'
import { diskNodeActions, driveActions } from '@/features/disk/actions'
import { gameActions } from '@/features/games/actions'
import { dayActions } from '@/features/life/actions'
import { projectActions } from '@/features/projects/actions'
import { settingActions } from '@/features/settings/actions'
import { idleJobActions, jobActions, sessionActions } from '@/features/workflows/actions'
import { commandActions } from './commands'
import type { ActionContext, ActionProvider, ActionTarget, ItemType, RainAction, TargetOf } from './types'

const providers: { [K in ItemType]: ActionProvider<TargetOf<K>> } = {
  app: appActions,
  pool: poolActions,
  project: projectActions,
  game: gameActions,
  session: sessionActions,
  job: jobActions,
  idleJob: idleJobActions,
  command: commandActions,
  day: dayActions,
  drive: driveActions,
  diskNode: diskNodeActions,
  setting: settingActions,
}

/**
 * Every action for the target, in display order.
 * Providers return exactly one 'primary' (Enter) and at most one 'secondary' (Ctrl+Enter);
 * an action's `keys` chord is live while its target is selected.
 */
export function actionsFor(target: ActionTarget, ctx: ActionContext): RainAction[] {
  // The table is typed per item type; indexing it with a union loses that pairing.
  const provider = providers[target.type] as ActionProvider
  return provider(target, ctx)
}
