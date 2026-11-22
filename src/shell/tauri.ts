// TauriShell: the real window. This is the only file that imports @tauri-apps/api,
// and it is loaded with a dynamic import so a plain browser never evaluates it.
//
// Split of work with src-tauri/src/lib.rs: Rust owns the global hotkey and tells
// us what it did through two events. Hide on blur lives here, because only the
// frontend knows the reasons to stay open (an overlay, a launch, a scan, ...).
import { listen } from '@tauri-apps/api/event'
import { getCurrentWindow } from '@tauri-apps/api/window'
import { readonly, ref } from 'vue'
import { createSignal, type Shell } from './shell'

/** Sent by Rust after it showed and focused the window (hotkey or second launch). */
const EVENT_SUMMONED = 'rain:summoned'
/** Sent by Rust after it hid the window (hotkey or Alt+F4). */
const EVENT_HIDDEN = 'rain:hidden'

export async function createTauriShell(): Promise<Shell> {
  const win = getCurrentWindow()
  // Rust shows the window before the page has loaded, so the first
  // 'rain:summoned' is never seen here. Ask instead of assuming.
  const hidden = ref(!(await win.isVisible()))
  const keepOpen = ref(false)
  // The window-state plugin restores full screen, so the window may start in it.
  const fullscreen = ref(await win.isFullscreen())
  const holds = new Set<symbol>()
  const summoned = createSignal()
  const hid = createSignal()

  function markHidden() {
    if (hidden.value) return
    hidden.value = true
    hid.emit()
  }

  function markSummoned() {
    hidden.value = false
    summoned.emit()
  }

  async function hide() {
    // Flag first: listeners stop their timers before the window goes away.
    markHidden()
    await win.hide()
  }

  function hidesOnBlur(): boolean {
    // Dev builds stay open so the window survives a click into DevTools or the editor.
    return !import.meta.env.DEV && !keepOpen.value && holds.size === 0 && !hidden.value
  }

  // The three listeners live as long as the page, so their unlisten functions are not kept.
  await listen(EVENT_SUMMONED, markSummoned)
  await listen(EVENT_HIDDEN, markHidden)
  await win.onFocusChanged(({ payload: focused }) => {
    if (!focused && hidesOnBlur()) void hide()
  })

  return {
    kind: 'tauri',
    hidden: readonly(hidden),
    keepOpen,

    async show() {
      await win.show()
      await win.setFocus()
      markSummoned()
    },

    hide,

    fullscreen: readonly(fullscreen),

    async toggleFullscreen() {
      await win.setFullscreen(!fullscreen.value)
      fullscreen.value = await win.isFullscreen()
    },

    holdOpen(reason) {
      // The reason is the symbol's description, so `holds` reads well in DevTools.
      const hold = Symbol(reason)
      holds.add(hold)
      return () => holds.delete(hold)
    },

    onSummon: summoned.on,
    onHide: hid.on,
  }
}
