// Keeps the selected item of a list or grid visible. Selection is moved with the
// keyboard while focus stays in the search field, so the browser does not scroll for us.
import { onMounted, watch } from 'vue'
import type { SelGroup } from '@/composables/useSelection'

/**
 * Scrolls the element marked `data-item-id="<id>"` inside `container` into view,
 * by the smallest distance and without animation. An item marked `data-group-start`
 * brings its group's header (`data-group-header`) along, so the header does not stay
 * hidden above the first row.
 */
export function revealItem(container: HTMLElement | null, id: string | null): void {
  if (!container || id === null) return
  const item = container.querySelector<HTMLElement>(`[data-item-id="${CSS.escape(id)}"]`)
  if (!item) return

  if ('groupStart' in item.dataset) {
    item.closest('[data-group]')?.querySelector('[data-group-header]')?.scrollIntoView({ block: 'nearest' })
  }
  item.scrollIntoView({ block: 'nearest' })
}

/**
 * Reveals the selected item on a selection change, and when a remembered selection
 * gets its element (data arrived). Other data updates are left alone, so a refresh
 * does not undo a scroll made with the mouse.
 */
export function useRevealSelected<T extends { id: string }>(
  container: () => HTMLElement | null,
  groups: () => SelGroup<T>[],
  selectedId: () => string | null,
): void {
  const selectedIsShown = () =>
    groups().some((group) => group.items.some((item) => item.id === selectedId()))

  watch([selectedId, selectedIsShown], () => revealItem(container(), selectedId()), { flush: 'post' })
  onMounted(() => revealItem(container(), selectedId()))
}
