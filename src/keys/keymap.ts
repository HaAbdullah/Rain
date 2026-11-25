// The single key table (spec 4.2). It binds the keys (useKeymap reads it), fills
// the tooltips and generates the shortcut sheet. Combos are written with
// KeyboardEvent.code names ('Ctrl+KeyK', 'Shift+Escape'), so the keyboard
// layout does not matter.
import { toggleKeepOpen } from '@/actions/common'
import { actionContext, runAction, runPrimary } from '@/actions/context'
import type { ActionTarget } from '@/actions/types'
import { useActionPanel } from '@/composables/useActionPanel'
import { useCheatSheet } from '@/composables/useCheatSheet'
import { refreshResources } from '@/composables/useResource'
import { resetZoom, zoomIn, zoomOut } from '@/composables/useZoom'
import { useScreenState, type ScreenController } from '@/composables/useScreen'
import type { useSearch } from '@/composables/useSearch'
import { poolEditor } from '@/features/apps/actions'
import { openAddGame } from '@/features/games/actions'
import { habitEditor, openTodaysNote } from '@/features/life/actions'
import { errorMessage } from '@/lib/errors'
import { routeByName, routes, type Route } from '@/nav/routes'
import { useNav } from '@/nav/useNav'
import { shell } from '@/shell'

export { formatCombo } from '@/lib/keys'

export type KeyGroup = 'Window' | 'Go' | 'Move' | 'Act' | 'Command'

export interface KeyContext {
  route: Route
  screen: ScreenController | null
  target: ActionTarget | null
  search: ReturnType<typeof useSearch>
}

export interface KeyBinding {
  id: string
  /** One combo or several that share the binding: 'Ctrl+KeyK', 'F11', ['F1', 'Ctrl+Slash']. */
  combo: string | string[]
  group: KeyGroup
  /** Shown in the shortcut sheet and in tooltips. */
  label: string
  /** The binding is live only while this is true. Not live = the key is left alone. */
  when?: (ctx: KeyContext) => boolean
  /**
   * `combo` is the one that was pressed (useful when the binding has several).
   * Return false to say "not handled": the key then keeps its normal effect
   * (Left/Right move the caret when the screen is a list).
   */
  run: (ctx: KeyContext, combo: string) => void | boolean
}

/** Every combo of a binding, as a list. */
export function combosOf(binding: KeyBinding): string[] {
  return Array.isArray(binding.combo) ? binding.combo : [binding.combo]
}

const MODIFIERS = ['Ctrl', 'Alt', 'Shift']

/** Puts a combo's modifiers in one fixed order, so two spellings of a chord compare equal. */
export function canonicalCombo(combo: string): string {
  const parts = combo.split('+')
  const held = MODIFIERS.filter((modifier) => parts.includes(modifier))
  const key = parts.filter((part) => !MODIFIERS.includes(part))
  return [...held, ...key].join('+')
}

// On-screen keyboards, remote desktop and automation tools can send a key
// press with an empty `code`. These are the punctuation keys the table binds.
const CODE_BY_KEY: Record<string, string> = { '`': 'Backquote', ',': 'Comma', '/': 'Slash' }

/** The pressed combo in keymap notation. The Enter key of the number pad counts as Enter. */
export function comboOfEvent(event: KeyboardEvent): string {
  const physical = event.code || CODE_BY_KEY[event.key] || ''
  const code = physical === 'NumpadEnter' ? 'Enter' : physical
  const held = [event.ctrlKey && 'Ctrl', event.altKey && 'Alt', event.shiftKey && 'Shift']
  return [...held.filter((modifier) => modifier !== false), code].join('+')
}

// ---- helpers the bindings share ----

const screenState = useScreenState()
const nav = useNav()

/** The selected item's action that listens to this chord, if it has one. */
export function chordAction(combo: string) {
  return screenState.actions.value.find(
    (action) => action.keys !== undefined && canonicalCombo(action.keys) === combo,
  )
}

/** A table entry for a chord that belongs to the selected item's actions (Ctrl+P, Ctrl+Y, ...). */
function actionChord(id: string, combo: string, label: string): KeyBinding {
  return {
    id,
    combo,
    group: 'Act',
    label,
    when: () => chordAction(combo) !== undefined,
    run() {
      const action = chordAction(combo)
      if (action) void runAction(action)
    },
  }
}

function cycleSubView(screen: ScreenController | null, step: -1 | 1) {
  const subView = screen?.subView
  if (!subView) return
  const { ids, current } = subView
  const next = (ids.indexOf(current.value) + step + ids.length) % ids.length
  current.value = ids[next]
}

/**
 * Esc on the Game mode screen cancels game mode and returns to the previous
 * screen (spec 2.3). Once the wind-down has started there is nothing to cancel.
 */
async function cancelGameMode() {
  const { api, notice } = actionContext()
  try {
    const { phase } = await api.getGameMode()
    if (phase === 'windingDown' || phase === 'done') return
    if (phase !== 'off') {
      await api.resolveGameMode('cancel')
      notice.info('Game mode cancelled')
    }
  } catch (reason) {
    notice.error(errorMessage(reason, 'Game mode was not cancelled'))
    // A backend that is away has nothing to cancel: leave the screen. A refusal (conflict) keeps it.
    if (typeof reason === 'object' && reason !== null && 'code' in reason && reason.code === 'conflict') return
  }
  nav.back()
}

/** The Esc ladder below "close the overlay": leave a sub-view, clear the query, else hide (spec 2.3). */
function escape({ route, search, screen }: KeyContext) {
  if (screen?.back?.()) return
  if (search.query.value !== '') search.clear()
  else if (route.name === 'gameMode') void cancelGameMode()
  else void shell.hide()
}

/**
 * History back, as in a file manager or a browser: leave what is open inside the screen (the open
 * habit, the folder on Disk), else return to the screen before. Unlike Esc it never hides Rain.
 */
function historyBack({ screen }: KeyContext) {
  if (screen?.back?.() || screen?.stepOut?.()) return
  nav.stepBack()
}

// ---- the table ----

/**
 * Ctrl+` in the browser shell. In Tauri the chord is a global hotkey owned by
 * Rust and never reaches the page; there this entry only feeds the shortcut sheet.
 */
export const summonBinding: KeyBinding = {
  id: 'summon',
  combo: 'Ctrl+Backquote',
  group: 'Window',
  label: 'Summon / hide',
  run: () => void (shell.hidden.value ? shell.show() : shell.hide()),
}

const goBindings: KeyBinding[] = routes.map((def) => ({
  id: `go-${def.name}`,
  combo: def.combo,
  group: 'Go',
  label: def.title,
  run: () => nav.go(def.name),
}))

/**
 * Back and Forward are actions; the inputs below are only what sends them. `MouseBack` and
 * `MouseForward` are the side buttons of a mouse (useKeymap turns the button into this name),
 * `BrowserBack` and `BrowserForward` the keys some keyboards have.
 */
const historyBindings: KeyBinding[] = [
  {
    id: 'history-back',
    combo: ['Alt+ArrowLeft', 'BrowserBack', 'MouseBack'],
    group: 'Go',
    label: 'Back',
    run: historyBack,
  },
  {
    id: 'history-forward',
    combo: ['Alt+ArrowRight', 'BrowserForward', 'MouseForward'],
    group: 'Go',
    label: 'Forward',
    run: () => void nav.stepForward(),
  },
]

const ALT_DIGITS = ['1', '2', '3', '4', '5', '6', '7', '8', '9'].map((digit) => `Alt+Digit${digit}`)

export const keymap: KeyBinding[] = [
  // Window
  summonBinding,
  { id: 'back', combo: 'Escape', group: 'Window', label: 'Back: overlay, open habit, query, hide', run: escape },
  {
    id: 'hide',
    combo: 'Shift+Escape',
    group: 'Window',
    label: 'Hide now',
    run: () => void shell.hide(),
  },
  {
    id: 'cheat-sheet',
    combo: ['F1', 'Ctrl+Slash'],
    group: 'Window',
    label: 'Shortcut sheet',
    run: () => useCheatSheet().open(),
  },
  {
    id: 'keep-open',
    combo: 'F10',
    group: 'Window',
    label: 'Keep open on / off',
    run: () => toggleKeepOpen(actionContext()),
  },
  {
    id: 'maximize',
    combo: 'F11',
    group: 'Window',
    label: 'Maximise / restore',
    run: () => void shell.toggleMaximize(),
  },
  // Ctrl+Shift+Equal is the plus key of a layout that needs Shift for it.
  {
    id: 'zoom-in',
    combo: ['Ctrl+Equal', 'Ctrl+Shift+Equal', 'Ctrl+NumpadAdd'],
    group: 'Window',
    label: 'Zoom in',
    run: () => void zoomIn(),
  },
  {
    id: 'zoom-out',
    combo: ['Ctrl+Minus', 'Ctrl+NumpadSubtract'],
    group: 'Window',
    label: 'Zoom out',
    run: () => void zoomOut(),
  },
  {
    id: 'zoom-reset',
    combo: ['Ctrl+Digit0', 'Ctrl+Numpad0'],
    group: 'Window',
    label: 'Reset zoom',
    run: () => void resetZoom(),
  },

  // Go
  ...goBindings,
  ...historyBindings,
  {
    id: 'sub-view',
    combo: ['Tab', 'Shift+Tab'],
    group: 'Go',
    label: 'Next / previous sub-view',
    when: ({ screen }) => screen?.subView !== undefined,
    run: ({ screen }, combo) => cycleSubView(screen, combo === 'Tab' ? 1 : -1),
  },

  // Move
  {
    id: 'move-vertical',
    combo: ['ArrowUp', 'ArrowDown'],
    group: 'Move',
    label: 'Move selection',
    // Always handled, also without a screen: the caret must never jump to the line's end.
    run({ screen }, combo) {
      screen?.move(combo === 'ArrowUp' ? 'up' : 'down')
    },
  },
  {
    id: 'move-horizontal',
    combo: ['ArrowLeft', 'ArrowRight'],
    group: 'Move',
    label: 'Move selection (grids)',
    run: ({ screen }, combo) => screen?.move(combo === 'ArrowLeft' ? 'left' : 'right') ?? false,
  },
  {
    id: 'page',
    combo: ['PageUp', 'PageDown'],
    group: 'Move',
    label: 'Page up / down; a year in an open habit',
    run: ({ screen }, combo) => screen?.page?.(combo === 'PageUp' ? -1 : 1),
  },
  {
    id: 'group',
    combo: ['Ctrl+ArrowUp', 'Ctrl+ArrowDown'],
    group: 'Move',
    label: 'Previous / next group (Life: Daily, Weekly, Monthly)',
    when: ({ screen }) => screen?.group !== undefined,
    run: ({ screen }, combo) => screen?.group?.(combo === 'Ctrl+ArrowUp' ? -1 : 1),
  },
  {
    id: 'step-out',
    combo: 'Backspace',
    group: 'Move',
    label: 'Step out (empty query): folder up, close habit, else remove the chip',
    when: ({ search }) => search.query.value === '',
    run({ screen, search }) {
      if (!screen?.stepOut?.()) search.setScope(null)
    },
  },

  // Act
  {
    id: 'primary',
    combo: 'Enter',
    group: 'Act',
    label: 'Primary action',
    when: () => screenState.primary.value !== null,
    run() {
      if (screenState.primary.value) void runAction(screenState.primary.value)
    },
  },
  {
    id: 'secondary',
    combo: 'Ctrl+Enter',
    group: 'Act',
    label: 'Secondary action',
    when: () => screenState.secondary.value !== null,
    run() {
      if (screenState.secondary.value) void runAction(screenState.secondary.value)
    },
  },
  {
    id: 'actions',
    combo: 'Ctrl+KeyK',
    group: 'Act',
    label: 'Action panel',
    when: ({ target }) => target !== null,
    run({ target }) {
      if (target) useActionPanel().open(target)
    },
  },
  {
    id: 'digit',
    combo: ALT_DIGITS,
    group: 'Act',
    label: 'Run the Nth numbered row (Home)',
    when: ({ screen }) => screen?.digits !== undefined,
    run({ screen }, combo) {
      const target = screen?.digits?.value[ALT_DIGITS.indexOf(combo)]
      if (target) runPrimary(target)
    },
  },
  actionChord('pin', 'Ctrl+KeyP', 'Pin / unpin to Home'),
  {
    id: 'new',
    combo: 'Ctrl+KeyN',
    group: 'Act',
    label: 'New pool (Apps), custom game (Games), habit (Life)',
    when: () =>
      nav.displayed.value === 'apps' || nav.displayed.value === 'games' || nav.displayed.value === 'life',
    run() {
      if (nav.displayed.value === 'apps') poolEditor.create()
      else if (nav.displayed.value === 'life') habitEditor.create()
      else openAddGame()
    },
  },
  {
    id: 'run-now',
    combo: 'Ctrl+KeyR',
    group: 'Act',
    label: 'Run now / rescan / re-read; otherwise reload',
    run() {
      const action = chordAction('Ctrl+KeyR')
      if (action) void runAction(action)
      // No item-specific meaning: refetch what the screen shows.
      else refreshResources()
    },
  },
  actionChord('log', 'Ctrl+KeyL', 'Log or transcript'),
  actionChord('allow', 'Ctrl+KeyY', 'Allow once (waiting session)'),
  actionChord('deny', 'Ctrl+KeyD', 'Deny (waiting session)'),

  // Command
  {
    id: 'today-note',
    combo: 'Ctrl+KeyT',
    group: 'Command',
    label: "Open today's note, then hide",
    run: () =>
      void runAction({
        id: 'today-note',
        label: "Open today's note",
        icon: 'notebook-pen',
        slot: 'more',
        run: openTodaysNote,
      }),
  },
  {
    id: 'filter',
    combo: 'Ctrl+KeyF',
    group: 'Command',
    label: 'Section filter on / off',
    when: ({ route }) => routeByName[route.name].filterable,
    run({ route, search }) {
      search.setScope(search.scope.value === route.name ? null : route.name)
    },
  },
]
