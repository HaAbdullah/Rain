// Small helpers that decide how an item's actions are listed. The action panel
// and the shortcut sheet both use them, so the two lists agree.
import type { ActionTarget, RainAction } from './types'

/** The key to draw next to an action, or undefined when it has none. */
export function keysOf(action: Pick<RainAction, 'slot' | 'keys'>): string | undefined {
  if (action.slot === 'primary') return 'Enter'
  if (action.slot === 'secondary') return 'Ctrl+Enter'
  return action.keys
}

/** Danger actions go last. The sort is stable, so the registry's order holds otherwise. */
export function orderActions(actions: readonly RainAction[]): RainAction[] {
  return [...actions].sort((a, b) => Number(a.danger ?? false) - Number(b.danger ?? false))
}

/** A short name for the selected item, for a heading. Falls back to the item type. */
export function targetName(target: ActionTarget): string {
  const item = target.item as unknown as Record<string, unknown>
  for (const field of ['name', 'title', 'label']) {
    const value = item[field]
    if (typeof value === 'string' && value !== '') return value
  }
  return target.type
}
