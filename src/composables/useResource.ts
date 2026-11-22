// Cached async state, shared by key (spec 6.2). Home and a section that both ask
// for 'sessions' get the same refs, so one fetch feeds both and a change shows
// everywhere. A resource with a `live` topic also follows the api's push updates.
//
// Lifetime: an entry is "in use" while a component that asked for it is mounted.
// Unused entries keep their data, so coming back to a screen paints at once and
// then refreshes in the background.
import {
  computed,
  getCurrentScope,
  onScopeDispose,
  ref,
  shallowRef,
  triggerRef,
  type Ref,
} from 'vue'
import { api } from '@/data'
import type { RainError, Topic, TopicPayload } from '@/data/api'
import { errorMessage } from '@/lib/errors'

export interface Resource<T> {
  data: Ref<T | undefined>
  /** True during every fetch, also a background refresh. Feeds the header loader line. */
  loading: Ref<boolean>
  /** True once a first load has taken 150 ms. Never set while there is data to show. */
  showSkeleton: Ref<boolean>
  error: Ref<RainError | null>
  reload(): Promise<void>
}

interface Entry<T> extends Resource<T> {
  fetcher: () => Promise<T>
  users: number
  /** Number of the newest fetch; an older fetch that ends later is ignored. */
  latest: number
  unsubscribe: (() => void) | null
}

const SKELETON_DELAY_MS = 150

// The cache holds resources of many value types; `unknown` is narrowed again in useResource.
const cache = new Map<string, Entry<unknown>>()
const fetchesRunning = ref(0)

/** True while any resource is fetching (the 1 px loader line under the header). */
export const anyResourceLoading = computed(() => fetchesRunning.value > 0)

function toRainError(reason: unknown): RainError {
  if (typeof reason === 'object' && reason !== null && 'code' in reason && 'message' in reason) {
    return reason as RainError
  }
  return { code: 'failed', message: errorMessage(reason, String(reason)) }
}

/** Stores a fetched or pushed value as the newest truth for the entry. */
function accept<T>(entry: Entry<T>, value: T) {
  // A mock may hand back the same array after changing it in place; say "changed" anyway.
  if (entry.data.value === value) triggerRef(entry.data)
  else entry.data.value = value
  entry.error.value = null
}

function createEntry<T>(fetcher: () => Promise<T>): Entry<T> {
  const entry: Entry<T> = {
    data: shallowRef<T | undefined>(undefined),
    loading: ref(false),
    showSkeleton: ref(false),
    error: ref<RainError | null>(null),
    fetcher,
    users: 0,
    latest: 0,
    unsubscribe: null,

    async reload() {
      const mine = ++entry.latest
      entry.loading.value = true
      fetchesRunning.value += 1
      const skeletonTimer = window.setTimeout(() => {
        if (mine === entry.latest && entry.data.value === undefined) entry.showSkeleton.value = true
      }, SKELETON_DELAY_MS)

      try {
        const value = await entry.fetcher()
        if (mine === entry.latest) accept(entry, value)
      } catch (reason) {
        if (mine === entry.latest) entry.error.value = toRainError(reason)
      } finally {
        window.clearTimeout(skeletonTimer)
        fetchesRunning.value -= 1
        if (mine === entry.latest) {
          entry.loading.value = false
          entry.showSkeleton.value = false
        }
      }
    },
  }
  return entry
}

function attach<T>(entry: Entry<T>, live: Topic | undefined) {
  entry.users += 1
  // Checked for every user: the rail may hold 'sessions' without a topic before Workflows asks for one.
  if (live && !entry.unsubscribe) {
    entry.unsubscribe = api.subscribe(live, (payload: TopicPayload[Topic]) => {
      // A push is newer than any fetch still on its way, so that fetch is retired.
      entry.latest += 1
      entry.loading.value = false
      entry.showSkeleton.value = false
      // The caller pairs the key with the topic that carries the same value type.
      accept(entry, payload as T)
    })
  }
  if (entry.users === 1) void entry.reload()
}

function detach<T>(entry: Entry<T>) {
  entry.users -= 1
  if (entry.users > 0) return
  entry.unsubscribe?.()
  entry.unsubscribe = null
}

/**
 * Returns the shared resource for `key`, fetching it if nobody else has it in use.
 * Call it in a component's setup; called outside a component it stays in use for good.
 */
export function useResource<T>(
  key: string,
  fetcher: () => Promise<T>,
  opts?: { live?: Topic },
): Resource<T> {
  let entry = cache.get(key) as Entry<T> | undefined
  if (!entry) {
    entry = createEntry(fetcher)
    cache.set(key, entry as Entry<unknown>)
  }
  const shared = entry
  // The newest fetcher wins: keys like 'habitDays:<from>:<to>' close over their range.
  shared.fetcher = fetcher
  attach(shared, opts?.live)
  if (getCurrentScope()) onScopeDispose(() => detach(shared))
  return shared
}

function forget(key: string, entry: Entry<unknown>) {
  // Bumping `latest` makes a fetch that is still running throw its result away.
  entry.latest += 1
  cache.delete(key)
}

/** Empties the cache and refetches what is on screen. For a scenario change and "Reset mock data". */
export function resetResources(): void {
  for (const [key, entry] of cache) {
    if (entry.users === 0) {
      forget(key, entry)
      continue
    }
    entry.data.value = undefined
    entry.error.value = null
    void entry.reload()
  }
}

/**
 * Refetches after a change, keeping the current data on screen meanwhile.
 * A name matches its own key and the keys that extend it ('habitDays' matches
 * 'habitDays:2026-11-01:2026-11-30'). Without names: everything in use.
 */
export function refreshResources(...names: string[]): void {
  for (const [key, entry] of cache) {
    const wanted =
      names.length === 0 || names.some((name) => key === name || key.startsWith(`${name}:`))
    if (!wanted) continue
    if (entry.users > 0) void entry.reload()
    // Not on screen: dropping it is enough, the next user fetches fresh data.
    else if (names.length > 0) forget(key, entry)
  }
}
