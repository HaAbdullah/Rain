// The one keyboard handler (spec 4.4). A single capture-phase listener on
// `window` looks the pressed combo up in the key table. Screens never listen
// for keys themselves; they receive moves through their ScreenController.
import { useEventListener } from '@vueuse/core'
import { ref } from 'vue'
import { runAction } from '@/actions/context'
import { useNotice } from '@/composables/useNotice'
import { isOverlayOpen } from '@/composables/useOverlay'
import { useScreenState } from '@/composables/useScreen'
import { searchFieldElement, useSearch } from '@/composables/useSearch'
import { useNav } from '@/nav/useNav'
import { shell } from '@/shell'
import {
  canonicalCombo,
  chordAction,
  combosOf,
  comboOfEvent,
  keymap,
  summonBinding,
  type KeyBinding,
  type KeyContext,
} from './keymap'
import { useSummon } from './summon'

/** True while Alt is down. ListRow shows its Alt digit only meanwhile. */
export const altHeld = ref(false)

/**
 * Keys that WebView2 (or a browser) would act on by itself: print, reload,
 * find, zoom, history, caret browsing. They are swallowed even when Rain has
 * no use for them at the moment.
 */
const BLOCKED = new Set([
  'Ctrl+KeyP',
  'Ctrl+KeyR',
  'F5',
  'Ctrl+KeyF',
  'F3',
  'Ctrl+KeyG',
  'Ctrl+KeyN',
  'Ctrl+KeyT',
  'Ctrl+KeyL',
  'Ctrl+KeyD',
  'Ctrl+KeyJ',
  'Ctrl+KeyU',
  'F7',
  'Alt+ArrowLeft',
  'Alt+ArrowRight',
  // Zoom: Ctrl with plus, minus or zero, on the main block and the number pad.
  'Ctrl+Equal',
  'Ctrl+Shift+Equal',
  'Ctrl+NumpadAdd',
  'Ctrl+Minus',
  'Ctrl+Shift+Minus',
  'Ctrl+NumpadSubtract',
  'Ctrl+Digit0',
  'Ctrl+Numpad0',
])

/** Tab must not walk focus through the page, but inside an overlay it belongs to Reka. */
const BLOCKED_OUTSIDE_OVERLAYS = new Set(['Tab', 'Shift+Tab'])

/** DevTools stay reachable in dev builds only. */
const BLOCKED_IN_RELEASE = new Set(['F12', 'Ctrl+Shift+KeyI'])

/** The movement keys; every other bound key acts once per press. */
const REPEATABLE = new Set([
  'ArrowUp',
  'ArrowDown',
  'ArrowLeft',
  'ArrowRight',
  'PageUp',
  'PageDown',
  'Ctrl+ArrowUp',
  'Ctrl+ArrowDown',
])

/** MouseEvent.button of the two side buttons, as the names the key table uses. */
const MOUSE_COMBOS: Record<number, string> = { 3: 'MouseBack', 4: 'MouseForward' }

const MODIFIER_KEYS = new Set(['Control', 'Shift', 'Alt', 'Meta'])

function isBlocked(combo: string, inOverlay: boolean): boolean {
  if (BLOCKED.has(combo)) return true
  if (!inOverlay && BLOCKED_OUTSIDE_OVERLAYS.has(combo)) return true
  return !import.meta.env.DEV && BLOCKED_IN_RELEASE.has(combo)
}

/** Every binding by each of its combos. Several bindings may share one; `when` picks. */
const byCombo = new Map<string, KeyBinding[]>()
for (const binding of keymap) {
  for (const combo of combosOf(binding)) {
    const key = canonicalCombo(combo)
    byCombo.set(key, [...(byCombo.get(key) ?? []), binding])
  }
}

function isTextField(target: EventTarget | null): target is HTMLElement {
  return target instanceof HTMLElement && target.matches('input, textarea, select, [contenteditable="true"]')
}

/** Call once, from AppShell. */
export function useKeymap(): void {
  const nav = useNav()
  const search = useSearch()
  const notice = useNotice()
  const screenState = useScreenState()

  useSummon()

  function context(): KeyContext {
    return {
      route: nav.route.value,
      screen: screenState.screen.value,
      target: screenState.target.value,
      search,
    }
  }

  /** Runs what the combo is bound to. Returns true when the key was handled. */
  function dispatch(combo: string, only?: (binding: KeyBinding) => boolean): boolean {
    const ctx = context()
    const binding = byCombo
      .get(combo)
      ?.find((candidate) => (only?.(candidate) ?? true) && (candidate.when?.(ctx) ?? true))
    if (binding) return binding.run(ctx, combo) !== false

    // A chord of the selected item's actions that the table does not list.
    const action = only ? undefined : chordAction(combo)
    if (action) void runAction(action)
    return action !== undefined
  }

  /** A Settings field has focus: it keeps its keys, except Esc and the section chords. */
  function handleInField(event: KeyboardEvent, combo: string, field: HTMLElement) {
    if (combo === 'Escape') {
      event.preventDefault()
      field.blur()
      search.focus()
      return
    }
    const isSectionChord = combo.startsWith('Ctrl+Digit')
    // Zoom works in a field too: a text field has no use for these chords.
    const handled = dispatch(
      combo,
      (binding) => (isSectionChord && binding.group === 'Go') || binding.id.startsWith('zoom-'),
    )
    if (handled || isBlocked(combo, false)) event.preventDefault()
  }

  function onKeydown(event: KeyboardEvent) {
    if (event.isComposing) return
    altHeld.value = event.altKey
    const combo = comboOfEvent(event)

    // The browser stand-in for the global hotkey. It works in every state,
    // also "hidden" and with an overlay open, like the real one.
    if (shell.kind === 'browser' && combo === summonBinding.combo) {
      event.preventDefault()
      if (!event.repeat) summonBinding.run(context(), combo)
      return
    }
    // The hidden placeholder is showing (browser only): nothing else is live.
    if (shell.hidden.value) return
    if (MODIFIER_KEYS.has(event.key) || event.metaKey) return

    // An error notice stays until the next key press (spec 3.6).
    if (notice.notice.value?.kind === 'error') notice.clear()

    // Hide at once, from any state. Reka would close an open overlay first, so it is not let through.
    if (combo === 'Shift+Escape') {
      event.preventDefault()
      event.stopPropagation()
      if (!event.repeat) void shell.hide()
      return
    }

    if (isOverlayOpen(event.target)) {
      if (isBlocked(combo, true)) event.preventDefault()
      return
    }

    const field = searchFieldElement()
    if (isTextField(event.target) && event.target !== field) {
      handleInField(event, combo, event.target)
      return
    }

    // One step per press: a held key repeats only where repeating is the point.
    if (event.repeat && !REPEATABLE.has(combo)) {
      // Backspace keeps its text effect, so a held key still erases the query.
      if (combo !== 'Backspace' && (byCombo.has(combo) || isBlocked(combo, false))) event.preventDefault()
      return
    }

    // Plain keys always type into the search field, wherever focus was left.
    if (!search.disabled.value && document.activeElement !== field) search.focus()

    if (dispatch(combo) || isBlocked(combo, false)) event.preventDefault()
  }

  function onKeyup(event: KeyboardEvent) {
    altHeld.value = event.altKey
    // A lone Alt would otherwise move focus to the (absent) window menu.
    if (event.key === 'Alt') event.preventDefault()
  }

  /**
   * The side buttons of a mouse send Back and Forward. They run the same two bindings as the
   * keys; WebView2's own history step is stopped (there is no page to go back to).
   */
  function onMouseup(event: MouseEvent) {
    const combo = MOUSE_COMBOS[event.button]
    if (!combo) return
    event.preventDefault()
    if (shell.hidden.value || isOverlayOpen(event.target)) return
    dispatch(combo)
  }

  useEventListener(window, 'mouseup', onMouseup, { capture: true })
  useEventListener(window, 'keydown', onKeydown, { capture: true })
  useEventListener(window, 'keyup', onKeyup, { capture: true })
  // Alt+Tab away: the key-up goes to the other window.
  useEventListener(window, 'blur', () => (altHeld.value = false))
}
