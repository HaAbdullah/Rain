// BrowserShell: the stand-in used by `npm run dev` in a plain browser tab.
// "Hidden" is only a flag; AppShell renders HiddenPlaceholder while it is set,
// so the summon rules can be tried without Tauri (spec 2.7).
import { readonly, ref } from 'vue'
import { createSignal, type Shell } from './shell'

export function createBrowserShell(): Shell {
  const hidden = ref(false)
  const keepOpen = ref(false)
  const fullscreen = ref(document.fullscreenElement !== null)
  // The browser can also leave full screen by itself (Esc), so follow its event.
  document.addEventListener('fullscreenchange', () => {
    fullscreen.value = document.fullscreenElement !== null
  })
  const summoned = createSignal()
  const hid = createSignal()

  return {
    kind: 'browser',
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

    fullscreen: readonly(fullscreen),

    async toggleFullscreen() {
      if (document.fullscreenElement) await document.exitFullscreen()
      else await document.documentElement.requestFullscreen()
    },

    // The browser never hides on blur, so there is nothing to suspend.
    holdOpen() {
      return () => {}
    },

    onSummon: summoned.on,
    onHide: hid.on,
  }
}
