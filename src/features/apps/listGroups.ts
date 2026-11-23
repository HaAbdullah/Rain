// The two groups of the Apps list (spec 7.2): pools first, then apps. Kept apart from the screen so
// that the counts in the group headers can be tested.
import type { AppEntry, Pool } from '@/data/types'
import { matches } from '@/lib/score'
import type { SelGroup } from '@/composables/useSelection'

/** A list row. Pool and app ids may be equal, so the row id carries the kind. */
export type Row =
  | { id: string; kind: 'pool'; pool: Pool }
  | { id: string; kind: 'app'; app: AppEntry }
  /** The dashed "New pool" row, shown only while there are no pools. */
  | { id: string; kind: 'new' }

export const NEW_POOL_ROW: Row = { id: 'new-pool', kind: 'new' }

/**
 * `pools` is `undefined` until the first answer. The "New pool" row is an invitation, not a pool:
 * it appears in the list but is never part of the count in the header.
 */
export function buildGroups(pools: Pool[] | undefined, apps: AppEntry[], query: string): SelGroup<Row>[] {
  const poolRows: Row[] = (pools ?? [])
    .filter((pool) => matches(query, pool.name))
    .map((pool) => ({ id: `pool:${pool.id}`, kind: 'pool', pool }))
  const poolCount = poolRows.length
  // With no pools at all (not merely none that match) the group offers to make one.
  if (pools?.length === 0 && query === '') poolRows.push(NEW_POOL_ROW)

  const appRows: Row[] = apps
    .filter((app) => matches(query, app.name, app.keywords))
    .sort((a, b) => a.name.localeCompare(b.name))
    .map((app) => ({ id: `app:${app.id}`, kind: 'app', app }))

  return [
    { id: 'pools', title: 'Pools', count: poolCount, items: poolRows },
    { id: 'apps', title: 'Apps', count: appRows.length, items: appRows },
  ]
}
