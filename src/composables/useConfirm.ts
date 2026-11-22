// Opener for the confirm dialog. `useConfirm()` gives the function an action
// awaits; `useConfirmState()` is what the ConfirmDialog component (mounted once
// in AppShell) draws and answers.
import { ref, shallowRef, watch, type Ref } from 'vue'
import { useOverlay } from './useOverlay'

export interface ConfirmOptions {
  title: string
  body: string
  confirmLabel: string
  danger?: boolean
}

const isOpen = ref(false)
const options = shallowRef<ConfirmOptions | null>(null)
let settle: ((confirmed: boolean) => void) | null = null

useOverlay(isOpen)

function answer(confirmed: boolean) {
  const resolve = settle
  settle = null
  isOpen.value = false
  resolve?.(confirmed)
}

// Closed some other way (Esc, a click outside): that is a "no".
watch(isOpen, (open) => {
  if (!open) answer(false)
})

function confirm(next: ConfirmOptions): Promise<boolean> {
  // A question that is still open is dropped in favour of the new one.
  answer(false)
  options.value = next
  isOpen.value = true
  return new Promise((resolve) => {
    settle = resolve
  })
}

/** Returns the function that asks: `if (await confirm({ ... })) ...`. */
export function useConfirm(): (options: ConfirmOptions) => Promise<boolean> {
  return confirm
}

export interface ConfirmState {
  isOpen: Ref<boolean>
  options: Readonly<Ref<ConfirmOptions | null>>
  /** The dialog calls this from its Cancel and Confirm buttons. */
  answer(confirmed: boolean): void
}

const state: ConfirmState = { isOpen, options, answer }

export function useConfirmState(): ConfirmState {
  return state
}
