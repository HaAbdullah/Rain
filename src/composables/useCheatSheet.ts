// Opener for the shortcut sheet (F1, Ctrl+/, the "Shortcut sheet" command).
// The CheatSheet component (mounted once in AppShell) draws it.
import { ref, type Ref } from 'vue'
import { useOverlay } from './useOverlay'

const isOpen = ref(false)

useOverlay(isOpen)

export interface CheatSheetStore {
  isOpen: Ref<boolean>
  open(): void
  close(): void
}

const store: CheatSheetStore = {
  isOpen,
  open() {
    isOpen.value = true
  },
  close() {
    isOpen.value = false
  },
}

export function useCheatSheet(): CheatSheetStore {
  return store
}
