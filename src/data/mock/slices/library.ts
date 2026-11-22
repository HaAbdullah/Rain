// Mock slice: projects and the game library.

import type { RainApi } from '../../api'
import type { Game, GameSource } from '../../types'
import { clone, delay, fail, findOrFail, slug, uniqueId } from '../engine'
import { coverFor } from '../covers'
import { options } from '../options'
import { sourceEnabled, state } from '../store'

type LibrarySlice = Pick<RainApi, 'listProjects' | 'listGames' | 'saveCustomGame' | 'setGameHidden'>

export const librarySlice: LibrarySlice = {
  async listProjects() {
    await delay()
    return clone(state.projects)
  },

  async listGames() {
    await delay()
    if (options.scenario === 'failing') throw fail('unavailable', 'Game libraries unavailable', 'games')

    const failedSources: GameSource[] = []
    if (state.settings.epicEnabled && state.epicFailuresLeft > 0) {
      state.epicFailuresLeft -= 1
      failedSources.push('epic')
    }
    const games = state.games.filter((game) => sourceEnabled(game.source) && !failedSources.includes(game.source))
    return { games: clone(games), failedSources }
  },

  async saveCustomGame(draft) {
    await delay()
    const title = draft.title.trim()
    if (title === '') throw fail('failed', 'A game needs a name')

    const game: Game = {
      id: uniqueId(`game-${slug(title)}`, state.games.map((existing) => existing.id)),
      title,
      source: 'custom',
      // The mock cannot read the chosen picture, so a cover path gets a generated cover.
      coverUrl: draft.coverPath?.trim() ? coverFor(title) : undefined,
      installed: true,
      running: false,
      playtimeMinutes: 0,
      pinned: false,
      hidden: false,
    }
    state.games.push(game)
    return clone(game)
  },

  async setGameHidden(id, hidden) {
    await delay()
    const game = findOrFail(state.games, id, 'Game')
    game.hidden = hidden
    if (hidden) {
      // A hidden game must not stay on Home.
      game.pinned = false
      state.pins = state.pins.filter((pin) => !(pin.type === 'game' && pin.id === id))
    }
  },
}
