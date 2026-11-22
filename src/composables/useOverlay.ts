// The registry of open overlays (action panel, dialogs, feature dialogs).
// While any overlay is open the global keymap stands down (Reka handles keys
// inside it) and the window does not hide on blur. When an overlay closes,
// the caret goes back to the search field.
import { computed, getCurrentScope, onScopeDispose, ref, watch, type Ref } from 'vue'
import { shell } from '@/shell'
import { useSearch } from './useSearch'

const openCount = ref(0)

/** True while at least one registered overlay is open. */
export const anyOverlayOpen = computed(() => openCount.value > 0)

/**
 * True while a Reka layer (dialog, popover, open select) is up. The registry
 * covers the app's own overlays; the DOM test also catches a Select that a
 * screen opened without registering it. `target` is where the event happened.
 */
export function isOverlayOpen(target: EventTarget | null): boolean {
  if (anyOverlayOpen.value) return true
  const layer = '[data-dismissable-layer], [data-reka-popper-content-wrapper]'
  if (target instanceof Element && target.closest(layer)) return true
  return document.activeElement?.closest(layer) != null
}

/**
 * Registers an overlay by its open flag. Call it once where the flag is created:
 * in a dialog component's setup, or at module level for an app-wide overlay.
 */
export function useOverlay(open: Ref<boolean>): void {
  const search = useSearch()
  let release: (() => void) | null = null

  function opened() {
    if (release) return
    openCount.value += 1
    release = shell.holdOpen('overlay')
  }

  function closed() {
    if (!release) return
    openCount.value -= 1
    release()
    release = null
    // After Reka has given focus back to whatever opened the overlay,
    // which may be a button; the search field is where focus rests.
    window.setTimeout(() => {
      if (!anyOverlayOpen.value) search.focus()
    })
  }

  const stop = watch(open, (isOpen) => (isOpen ? opened() : closed()), { immediate: true })

  if (getCurrentScope()) {
    onScopeDispose(() => {
      stop()
      closed()
    })
  }
}
