// The mock backend: one RainApi put together from the slices. All of them work
// on the single in-memory state in store.ts, so a change made through one call
// shows up in every other call and on the matching subscribe() topic.

import type { RainApi } from '../api'
import { subscribe } from './engine'
import { diskSlice } from './slices/disk'
import { gameModeSlice } from './slices/gameMode'
import { jobsSlice } from './slices/jobs'
import { launcherSlice } from './slices/launcher'
import { launchesSlice } from './slices/launches'
import { librarySlice } from './slices/library'
import { lifeSlice } from './slices/life'
import { sessionsSlice, startSessionActivity } from './slices/sessions'
import { settingsSlice } from './slices/settings'
import { onStateReset } from './store'

/**
 * The methods the mock no longer implements, one name per method (foundation 8.6). A stage that
 * moves a feature to the backend and deletes its mock slice adds that slice's method names here
 * in the same commit, so `MockApi` stays true to what `createMockApi()` really returns.
 * `getHealth` is the first: it is new, and the mock never had it.
 */
export type RetiredMockMethod =
  | 'getHealth'
  // A section adds its methods below its own comment, one per line.
  // launcher and apps (section 01)
  // projects and games (section 02)
  // sessions and jobs (section 03)
  // life (section 04)
  // disk and machine (section 05)
  // game mode (section 06)

/** The part of `RainApi` the mock still implements. */
export type MockApi = Omit<RainApi, RetiredMockMethod>

export function createMockApi(): MockApi {
  // A reseed cancels every timer, so the background activity is started again after it.
  startSessionActivity()
  onStateReset(startSessionActivity)

  return {
    ...launcherSlice,
    ...launchesSlice,
    ...librarySlice,
    ...sessionsSlice,
    ...jobsSlice,
    ...lifeSlice,
    ...diskSlice,
    ...gameModeSlice,
    ...settingsSlice,
    subscribe,
  }
}
