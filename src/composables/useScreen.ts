// The one contract between a screen and the shell (spec 6.2). A screen hands over
// a ScreenController; the keymap sends it arrow keys, and Enter and the Ctrl+K
// panel read the selected target from it. Only one screen is mounted at a
// time, so one module-level slot is enough.
import { computed, onScopeDispose, shallowRef, type ComputedRef, type Ref } from 'vue'
import { actionContext } from '@/actions/context'
import { actionsFor } from '@/actions/registry'
import type { ActionTarget, RainAction } from '@/actions/types'

export type Dir = 'up' | 'down' | 'left' | 'right'

export interface ScreenController {
  /** The selected item: what Enter, Ctrl+K and the chords act on. */
  target: Ref<ActionTarget | null>
  /** Returns false when not handled, which lets Left/Right reach the caret. */
  move(dir: Dir): boolean
  page?(dir: -1 | 1): void
  group?(dir: -1 | 1): void
  /** Tab cycles through these. */
  subView?: { ids: readonly string[]; current: Ref<string> }
  /** Backspace on an empty query. Returns false when not handled. */
  stepOut?(): boolean
  /** Esc: leave a sub-view of the screen (Life: the open habit). Returns false when there is none. */
  back?(): boolean
  /** Alt+1..9 targets (Home only). */
  digits?: Ref<ActionTarget[]>
  /** Game mode: replaces the registry's actions. */
  primaryOverride?: Ref<RainAction[] | null>
}

const active = shallowRef<ScreenController | null>(null)

/** Call once in each screen's setup. */
export function useScreen(controller: ScreenController): void {
  active.value = controller
  onScopeDispose(() => {
    // The next screen may already have registered; only clear our own entry.
    if (active.value === controller) active.value = null
  })
}

export interface ScreenState {
  screen: Readonly<Ref<ScreenController | null>>
  target: ComputedRef<ActionTarget | null>
  /** Every action for the current target (or the screen's override), in registry order. */
  actions: ComputedRef<RainAction[]>
  primary: ComputedRef<RainAction | null>
  secondary: ComputedRef<RainAction | null>
}

const target = computed(() => active.value?.target.value ?? null)
const actions = computed<RainAction[]>(() => {
  const override = active.value?.primaryOverride?.value
  if (override) return override
  return target.value ? actionsFor(target.value, actionContext()) : []
})
const primary = computed(() => actions.value.find((action) => action.slot === 'primary') ?? null)
const secondary = computed(
  () => actions.value.find((action) => action.slot === 'secondary') ?? null,
)

const state: ScreenState = { screen: active, target, actions, primary, secondary }

/** What the shell reads: the active screen and what can be done to its selection. */
export function useScreenState(): ScreenState {
  return state
}
