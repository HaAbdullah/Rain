// Mock slice: search, pins, recent items, apps and pools.

import type { RainApi } from '../../api'
import type { ID, ISODateTime, PinnableType, Pool, PoolMember, SearchItem } from '../../types'
import { clone, delay, fail, findOrFail, letter, slug, uniqueId } from '../engine'
import { appItem, gameItem, poolItem, projectItem, searchItems } from '../search'
import { findableGames, state } from '../store'

type LauncherSlice = Pick<RainApi,
  'search' | 'listPinned' | 'listRecent' | 'setPinned' | 'listApps' | 'listPools' | 'savePool' | 'deletePool'>

export const launcherSlice: LauncherSlice = {
  async search(query, types) {
    await delay('search')
    return searchItems(query, types)
  },

  async listPinned() {
    await delay()
    return state.pins.flatMap((pin) => itemFor(pin.type, pin.id) ?? [])
  },

  async listRecent(limit) {
    await delay()
    return recentItems().slice(0, limit)
  },

  async setPinned(type, id, pinned) {
    await delay()
    const entity = pinnable(type, id)
    if (!entity) throw fail('notFound', 'Item not found')
    entity.pinned = pinned
    state.pins = state.pins.filter((pin) => !(pin.type === type && pin.id === id))
    if (pinned) state.pins.push({ type, id })
  },

  async listApps() {
    await delay()
    return clone(state.apps)
  },

  async listPools() {
    await delay()
    return clone(state.pools)
  },

  async savePool(draft) {
    await delay()
    const name = draft.name.trim()
    if (name === '') throw fail('failed', 'A pool needs a name')
    if (draft.members.length === 0) throw fail('failed', 'A pool needs at least one app')

    // Copied field by field: the draft may be a reactive form object.
    const members: PoolMember[] = draft.members.map((member) => ({ appId: member.appId, args: member.args, delayMs: member.delayMs }))
    const pool = draft.id === undefined ? createPool(name) : findOrFail(state.pools, draft.id, 'Pool')
    pool.name = name
    pool.icon = letter(name)
    pool.members = members
    return clone(pool)
  },

  async deletePool(id) {
    await delay()
    findOrFail(state.pools, id, 'Pool')
    state.pools = state.pools.filter((pool) => pool.id !== id)
    state.pins = state.pins.filter((pin) => !(pin.type === 'pool' && pin.id === id))
  },
}

function createPool(name: string): Pool {
  const id = uniqueId(`pool-${slug(name)}`, state.pools.map((pool) => pool.id))
  const pool: Pool = { id, name, icon: letter(name), members: [], pinned: false }
  state.pools.push(pool)
  return pool
}

function pinnable(type: PinnableType, id: ID): { pinned: boolean } | undefined {
  switch (type) {
    case 'app': return state.apps.find((app) => app.id === id)
    case 'pool': return state.pools.find((pool) => pool.id === id)
    case 'project': return state.projects.find((project) => project.id === id)
    case 'game': return state.games.find((game) => game.id === id)
  }
}

function itemFor(type: PinnableType, id: ID): SearchItem | undefined {
  switch (type) {
    case 'app': return mapFound(state.apps, id, appItem)
    case 'pool': return mapFound(state.pools, id, poolItem)
    case 'project': return mapFound(state.projects, id, projectItem)
    case 'game': return mapFound(findableGames(), id, gameItem)
  }
}

function mapFound<T extends { id: ID }>(items: T[], id: ID, toItem: (entity: T) => SearchItem): SearchItem | undefined {
  const entity = items.find((candidate) => candidate.id === id)
  return entity && toItem(entity)
}

/** Everything that was ever launched, opened or played, newest first. */
function recentItems(): SearchItem[] {
  const used: { at: ISODateTime | undefined; item: SearchItem }[] = [
    ...state.apps.map((app) => ({ at: app.lastLaunchedAt, item: appItem(app) })),
    ...state.pools.map((pool) => ({ at: pool.lastLaunchedAt, item: poolItem(pool) })),
    ...state.projects.filter((project) => !project.missing).map((project) => ({ at: project.lastOpenedAt, item: projectItem(project) })),
    ...findableGames().map((game) => ({ at: game.lastPlayedAt, item: gameItem(game) })),
  ]
  return used
    .filter((entry): entry is { at: ISODateTime; item: SearchItem } => entry.at !== undefined)
    // ISO date-times sort correctly as text.
    .sort((a, b) => b.at.localeCompare(a.at))
    .map((entry) => entry.item)
}
