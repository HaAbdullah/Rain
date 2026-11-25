// The order of the sections in the left rail, as plain functions (no Vue, no storage).
// useRailOrder.ts keeps the order and the key of each place; this file only does the arithmetic.

/** Ctrl+1 .. Ctrl+9. Ctrl+0 is "reset zoom", so a tenth section has no key of its own. */
export const SECTION_KEY_COUNT = 9

/**
 * The stored order made to fit the sections that exist now: ids that are gone or repeated are
 * dropped, sections that are new (or never stored) are appended in their default order.
 * Anything that is not a list of text gives the default order.
 */
export function reconcileOrder<T extends string>(stored: unknown, ids: readonly T[]): T[] {
  const known = new Set<string>(ids)
  const kept: T[] = []
  if (Array.isArray(stored)) {
    for (const entry of stored) {
      if (typeof entry === 'string' && known.has(entry) && !kept.includes(entry as T)) kept.push(entry as T)
    }
  }
  return [...kept, ...ids.filter((id) => !kept.includes(id))]
}

/** The order after moving one id up (-1) or down (+1). At an end, or for an unknown id, it stays as it is. */
export function moveBy<T>(order: readonly T[], id: T, step: -1 | 1): T[] {
  const from = order.indexOf(id)
  const to = from + step
  if (from < 0 || to < 0 || to >= order.length) return [...order]
  const next = [...order]
  next[from] = order[to]
  next[to] = id
  return next
}

/**
 * The order after dropping an id into a slot. Slot 0 is above the first icon and slot `length`
 * below the last one, so a slot is a gap between icons (what a drag shows as a line).
 */
export function moveToSlot<T>(order: readonly T[], id: T, slot: number): T[] {
  const from = order.indexOf(id)
  if (from < 0) return [...order]
  const wanted = Math.min(Math.max(Math.round(slot), 0), order.length)
  const next = order.filter((entry) => entry !== id)
  next.splice(wanted > from ? wanted - 1 : wanted, 0, id)
  return next
}

/** The gap a pointer at height `y` is nearest to: the number of icons whose centre is above it. */
export function dropSlot(centres: readonly number[], y: number): number {
  return centres.filter((centre) => centre < y).length
}

/** Whether dropping an id into a slot would change anything. */
export function slotChangesOrder<T>(order: readonly T[], id: T, slot: number): boolean {
  const next = moveToSlot(order, id, slot)
  return next.some((entry, index) => entry !== order[index])
}

/** The key of the place at an index, counted from the top: 'Ctrl+Digit1' for the first. None after the ninth. */
export function sectionCombo(index: number): string | undefined {
  return index >= 0 && index < SECTION_KEY_COUNT ? `Ctrl+Digit${index + 1}` : undefined
}

export function sameOrder(a: readonly string[], b: readonly string[]): boolean {
  return a.length === b.length && a.every((entry, index) => entry === b[index])
}
