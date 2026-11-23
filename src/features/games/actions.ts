// Actions for a game (spec 4.3), and the hooks that open the add and edit dialog.
import { ref } from 'vue'
import { pinAction, revealAction } from '@/actions/common'
import type { ActionProvider, RainAction, TargetOf } from '@/actions/types'
import { refreshResources } from '@/composables/useResource'
import type { Game } from '@/data/types'

/**
 * CustomGameDialog (mounted by GamesScreen) is open while `open` is true. `editing` is the game
 * the dialog changes, or null when it adds one. Ctrl+N and "Edit game" set them.
 */
export const customGameDialog = { open: ref(false), editing: ref<Game | null>(null) }

export function openAddGame() {
  customGameDialog.editing.value = null
  customGameDialog.open.value = true
}

function openEditGame(game: Game) {
  customGameDialog.editing.value = game
  customGameDialog.open.value = true
}

/** A game that Enter can start. A local game needs a program file that exists. */
const hasProgram = (game: Game) => game.source !== 'custom' || (game.installed && game.exePath !== undefined)

function playAction(game: Game, slot: RainAction['slot']): RainAction {
  return {
    id: 'play',
    label: 'Play',
    icon: 'play',
    slot,
    run: async (ctx) => {
      await ctx.launch({ type: 'game', id: game.id }, game.title)
    },
  }
}

function chooseProgram(game: Game): RainAction {
  return {
    id: 'choose-program',
    label: 'Choose program',
    icon: 'folder-search',
    slot: 'primary',
    run: () => openEditGame(game),
  }
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

function editGame(game: Game): RainAction {
  return {
    id: 'edit',
    label: 'Edit game',
    icon: 'pencil',
    slot: 'more',
    run: () => openEditGame(game),
  }
}

/** Hide a visible game, or bring a hidden one back. */
function toggleHidden(game: Game, slot: RainAction['slot']): RainAction {
  return {
    id: game.hidden ? 'show' : 'hide',
    label: game.hidden ? 'Show in library' : 'Hide from library',
    icon: game.hidden ? 'eye' : 'eye-off',
    slot,
    async run({ api, notice }) {
      await api.setGameHidden(game.id, !game.hidden)
      refreshResources('games', 'pinned', 'recent')
      notice.success(game.hidden ? `${game.title} is back in the library` : `Hid ${game.title}`)
    },
  }
}

function removeGame(game: Game): RainAction {
  return {
    id: 'remove',
    label: 'Remove from library',
    icon: 'trash-2',
    slot: 'more',
    danger: true,
    async run({ api, confirm, notice }) {
      const confirmed = await confirm({
        title: `Remove ${game.title}?`,
        body: 'The game is taken out of Rain. Its files are not touched.',
        confirmLabel: 'Remove',
        danger: true,
      })
      if (!confirmed) return
      await api.deleteCustomGame(game.id)
      refreshResources('games', 'pinned', 'recent')
      notice.success(`Removed ${game.title}`)
    },
  }
}

export const gameActions: ActionProvider<TargetOf<'game'>> = ({ item: game }) => {
  // A hidden game can only be brought back (Enter) or played from its menu.
  if (game.hidden) {
    return [toggleHidden(game, 'primary'), ...(hasProgram(game) ? [playAction(game, 'more')] : [])]
  }
  const actions: RainAction[] = [hasProgram(game) ? playAction(game, 'primary') : chooseProgram(game)]
  if (hasProgram(game)) actions.push(playInGameMode(game))
  actions.push(pinAction('game', game.id, game.pinned))
  if (game.installPath) actions.push(revealAction(game.installPath, 'more', 'Show in Explorer'))
  if (game.source === 'custom') actions.push(editGame(game))
  actions.push(toggleHidden(game, 'more'))
  if (game.removable) actions.push(removeGame(game))
  return actions
}
