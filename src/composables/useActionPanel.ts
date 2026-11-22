// Opener for the Ctrl+K action panel. The state lives here; the ActionPanel
// component (mounted once in AppShell) draws it.
import { ref, shallowRef, type Ref } from 'vue'
import type { ActionTarget } from '@/actions/types'
import { useOverlay } from './useOverlay'

/** Viewport coordinates of a right-click. A MouseEvent can be passed as it is. */
export interface PanelAnchor {
  x: number
  y: number
}

const isOpen = ref(false)
const target = shallowRef<ActionTarget | null>(null)
const anchor = shallowRef<PanelAnchor | null>(null)

useOverlay(isOpen)

export interface ActionPanelStore {
  isOpen: Ref<boolean>
  target: Readonly<Ref<ActionTarget | null>>
  /** null = the default place (the bottom-right corner). */
  anchor: Readonly<Ref<PanelAnchor | null>>
  open(target: ActionTarget, anchor?: PanelAnchor): void
  close(): void
}

const store: ActionPanelStore = {
  isOpen,
  target,
  anchor,
  open(nextTarget, nextAnchor) {
    target.value = nextTarget
    // Copy the two numbers so a MouseEvent is not kept alive.
    anchor.value = nextAnchor ? { x: nextAnchor.x, y: nextAnchor.y } : null
    isOpen.value = true
  },
  close() {
    isOpen.value = false
  },
}

export function useActionPanel(): ActionPanelStore {
  return store
}
