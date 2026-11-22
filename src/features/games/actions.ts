// Actions for a game (spec 4.3), and the hook that opens the custom-game dialog.
import { ref } from 'vue'
import { pinAction } from '@/actions/common'
import type { ActionProvider, RainAction, TargetOf } from '@/actions/types'
import { refreshResources } from '@/composables/useResource'
import type { Game } from '@/data/types'

/** CustomGameDialog (mounted by GamesScreen) is open while this is true. Ctrl+N sets it. */
export const customGameDialog = { open: ref(false) }

const STORE_NAME: Record<Game['source'], string | null> = {
  steam: 'Steam',
  epic: 'Epic Games',
  custom: null,
}

function playInGameMode(game: Game): RainAction {
  return {
    id: 'play-game-mode',
    label: 'Play in game mode',
    icon: 'circle-power',
    slot: 'secondary',
    async run({ api, nav }) {
      // Started here, with the game attached, before the screen opens:
      // GameModeScreen only starts game mode itself when the phase is still 'off'.
      await api.startGameMode({ thenLaunchGameId: game.id })
      nav.go('gameMode')
    },
  }
}

function storePage(game: Game, store: string): RainAction {
  return {
    id: 'store',
    label: 'Open store page',
    icon: 'store',
    slot: 'more',
    // RainApi has no endpoint for this yet, so the prototype only reports it.
    run: ({ notice }) => notice.info(`${store} page of ${game.title} opened`),
  }
}

function toggleHidden(game: Game): RainAction {
  return {
    id: 'hide',
    label: game.hidden ? 'Show in library' : 'Hide from library',
    icon: game.hidden ? 'eye' : 'eye-off',
    slot: 'more',
    async run({ api, notice }) {
      await api.setGameHidden(game.id, !game.hidden)
      refreshResources('games')
      notice.success(game.hidden ? `${game.title} is back in the library` : `Hid ${game.title}`)
    },
  }
}

export const gameActions: ActionProvider<TargetOf<'game'>> = ({ item: game }) => {
  const store = STORE_NAME[game.source]
  const actions: RainAction[] = [
    {
      id: 'play',
      // Launching a game that is not installed starts its install instead.
      label: game.installed ? 'Play' : 'Install',
      icon: game.installed ? 'play' : 'download',
      slot: 'primary',
      run: async (ctx) => {
        await ctx.launch({ type: 'game', id: game.id }, game.title)
      },
    },
  ]
  if (game.installed) actions.push(playInGameMode(game))
  actions.push(pinAction('game', game.id, game.pinned))
  if (store) actions.push(storePage(game, store))
  actions.push(toggleHidden(game))
  return actions
}
