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

export function createMockApi(): RainApi {
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
