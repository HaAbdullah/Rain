// Mock slice: settings. A change applies at once and the other slices read it
// live: switching a game store off empties its part of the library, a different
// vault path disconnects the vault, the countdown length feeds game mode.

import type { RainApi } from '../../api'
import { clone, delay } from '../engine'
import { state } from '../store'

export const settingsSlice: Pick<RainApi, 'getSettings' | 'updateSettings'> = {
  async getSettings() {
    await delay()
    return clone(state.settings)
  },

  async updateSettings(patch) {
    await delay()
    Object.assign(state.settings, patch)
    // Copied so the mock never keeps hold of the caller's (possibly reactive) array.
    state.settings.projectRoots = [...state.settings.projectRoots]
    state.settings.gameFolders = [...state.settings.gameFolders]
    return clone(state.settings)
  },
}
