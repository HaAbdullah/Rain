// BrowserShell: the stand-in used by `npm run dev` in a plain browser tab.
// "Hidden" is only a flag; AppShell renders HiddenPlaceholder while it is set,
// so the summon rules can be tried without Tauri (spec 2.7).
import { readonly, ref } from 'vue'
import { createBrowserBridge } from './browser-bridge'
import { createSignal, type Shell } from './shell'

export function createBrowserShell(): Shell {
  const hidden = ref(false)
  const keepOpen = ref(false)
  // A tab cannot maximise; the browser's full screen is the closest stand-in.
  const maximized = ref(document.fullscreenElement !== null)
  // The browser can also leave full screen by itself (Esc), so follow its event.
  document.addEventListener('fullscreenchange', () => {
    maximized.value = document.fullscreenElement !== null
  })
  const summoned = createSignal()
  const hid = createSignal()

  return {
    kind: 'browser',
    bridge: createBrowserBridge(),
    hidden: readonly(hidden),
    keepOpen,

    async show() {
      hidden.value = false
      summoned.emit()
    },

    async hide() {
      if (hidden.value) return
      hidden.value = true
      hid.emit()
    },

    maximized: readonly(maximized),

    async toggleMaximize() {
      if (document.fullscreenElement) await document.exitFullscreen()
      else await document.documentElement.requestFullscreen()
    },

    // A tab has no work area of its own and no window to grow.
    async workArea() {
      return null
    },

    async applyZoom(percent) {
      document.documentElement.style.zoom = String(percent / 100)
    },

    // The browser never hides on blur, so there is nothing to suspend.
    holdOpen() {
      return () => {}
    },

    onSummon: summoned.on,
    onHide: hid.on,
  }
}
