// Turns a SearchItem (what search, pins and recents return: type + id + display
// text) into an ActionTarget (which carries the full entity the actions need).
// The entities come from the shared resource cache, so Home pays for no extra
// fetch when a section has already loaded its list.
import { useResource } from '@/composables/useResource'
import { api } from '@/data'
import type { ID, SearchItem, SearchType } from '@/data/types'
import type { ActionTarget } from './types'

/** Call in a component's setup. The returned function gives null while the entity's list is still loading. */
export function useTargetResolver(): (item: SearchItem) => ActionTarget | null {
  const apps = useResource('apps', () => api.listApps())
  const pools = useResource('pools', () => api.listPools())
  const projects = useResource('projects', () => api.listProjects())
  const games = useResource('games', () => api.listGames())
  const sessions = useResource('sessions', () => api.listSessions(), { live: 'sessions' })
  const jobs = useResource('jobs', () => api.listJobs(), { live: 'jobs' })
  const idleJobs = useResource('idleJobs', () => api.listIdleJobs(), { live: 'idleJobs' })

  function byId<T extends { id: ID }>(list: T[] | undefined, id: ID): T | undefined {
    return list?.find((entity) => entity.id === id)
  }

  const find: { [K in SearchType]: (id: ID) => ActionTarget | null } = {
    app: (id) => wrap('app', byId(apps.data.value, id)),
    pool: (id) => wrap('pool', byId(pools.data.value, id)),
    project: (id) => wrap('project', byId(projects.data.value, id)),
    game: (id) => wrap('game', byId(games.data.value?.games, id)),
    session: (id) => wrap('session', byId(sessions.data.value, id)),
    job: (id) => wrap('job', byId(jobs.data.value, id)),
    idleJob: (id) => wrap('idleJob', byId(idleJobs.data.value, id)),
  }

  return (item) => find[item.type](item.id)
}

/** Pairs an entity with its type tag; null when the entity was not found. */
function wrap<K extends SearchType>(
  type: K,
  item: Extract<ActionTarget, { type: K }>['item'] | undefined,
): ActionTarget | null {
  // The cast restores the type/item pairing that the generic K hides from the compiler.
  return item ? ({ type, id: item.id, item } as ActionTarget) : null
}
