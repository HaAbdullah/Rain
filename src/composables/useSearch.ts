// The one search field's state (spec 3.5): the query and the scope chip.
// scope === null means "search everything"; a route name means "filter that section".
// The state lives at module level, so every caller sees the same query.
import { computed, ref, type ComputedRef, type Ref } from 'vue'
import type { RouteName } from '@/nav/routes'

const query = ref('')
const scope = ref<RouteName | null>(null)
/** True while typing must not change what is shown (Game mode during review, countdown and wind-down). */
const disabled = ref(false)

/** The search input, registered by SearchField when it mounts. */
let field: HTMLInputElement | null = null

/** SearchField calls this with its <input> on mount and with null on unmount. */
export function registerSearchField(el: HTMLInputElement | null): void {
  field = el
}

/**
 * The search input, or null before SearchField has mounted.
 * Falls back to a `data-search-field` attribute so the keymap can still tell
 * the search field from a Settings field if the component did not register.
 */
export function searchFieldElement(): HTMLInputElement | null {
  if (field?.isConnected) return field
  return document.querySelector<HTMLInputElement>('input[data-search-field]')
}

export interface Search {
  query: Ref<string>
  scope: Ref<RouteName | null>
  /** A screen sets it while the search must stay as it is; the field is greyed out. */
  disabled: Ref<boolean>
  setScope(name: RouteName | null): void
  /** Empties the query. The scope chip stays. */
  clear(): void
  /** Puts the caret in the search field. */
  focus(): void
}

const search: Search = {
  query,
  scope,
  disabled,
  setScope(name) {
    scope.value = name
  },
  clear() {
    query.value = ''
  },
  focus() {
    // preventScroll: focusing must never move the content under the user.
    searchFieldElement()?.focus({ preventScroll: true })
  },
}

export function useSearch(): Search {
  return search
}

/** The query as one section sees it: '' unless the chip names that section. */
export function useSectionQuery(name: RouteName): ComputedRef<string> {
  return computed(() => (scope.value === name ? query.value : ''))
}
