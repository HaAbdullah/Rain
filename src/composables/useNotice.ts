// The one-line notice (shown by NoticePill at the bottom of the window): an
// action reports its result here. Info and success leave after 3 s; an error stays until
// the next key press (the keymap calls `clear`).
import { ref, type Ref } from 'vue'

export interface Notice {
  kind: 'info' | 'success' | 'error'
  text: string
}

const NOTICE_MS = 3000

const notice = ref<Notice | null>(null)
let timer: number | undefined

function show(kind: Notice['kind'], text: string) {
  window.clearTimeout(timer)
  notice.value = { kind, text }
  if (kind !== 'error') timer = window.setTimeout(clear, NOTICE_MS)
}

function clear() {
  window.clearTimeout(timer)
  notice.value = null
}

export interface Notices {
  notice: Ref<Notice | null>
  info(text: string): void
  success(text: string): void
  error(text: string): void
  clear(): void
}

const notices: Notices = {
  notice,
  info: (text) => show('info', text),
  success: (text) => show('success', text),
  error: (text) => show('error', text),
  clear,
}

export function useNotice(): Notices {
  return notices
}
