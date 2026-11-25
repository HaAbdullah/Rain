// TauriShell: the real window. Together with tauri-bridge.ts this is the only code that
// imports @tauri-apps/api, and it is loaded with a dynamic import so a plain browser
// never evaluates it.
//
// Split of work with src-tauri/src/lib.rs: Rust owns the global hotkey and tells
// us what it did through two events. Hide on blur lives here, because only the
// frontend knows the reasons to stay open (an overlay, a launch, a scan, ...).
import { listen } from '@tauri-apps/api/event'
import { getCurrentWebview } from '@tauri-apps/api/webview'
import { currentMonitor, getCurrentWindow, LogicalSize } from '@tauri-apps/api/window'
import { readonly, ref } from 'vue'
import { createSignal, MIN_LAYOUT, type Shell, type Size } from './shell'
import { createTauriBridge } from './tauri-bridge'

/** Sent by Rust after it showed and focused the window (hotkey or second launch). */
const EVENT_SUMMONED = 'rain:summoned'
/** Sent by Rust after it hid the window (hotkey or Alt+F4). */
const EVENT_HIDDEN = 'rain:hidden'

/**
 * The header carries data-tauri-drag-region. Tauri's own script then starts the drag on
 * mousedown and maximises on a double click. The CSS rule `app-region: drag` of the page
 * makes WebView2 take those mouse events itself, so the script never sees the second click.
 * Switching the CSS off leaves the work to the script.
 */
function leaveDragRegionsToTauri() {
  const style = document.createElement('style')
  style.textContent = '[data-tauri-drag-region] { app-region: no-drag !important; }'
  document.head.append(style)
}

export async function createTauriShell(): Promise<Shell> {
  leaveDragRegionsToTauri()
  const win = getCurrentWindow()
  // Rust shows the window before the page has loaded, so the first
  // 'rain:summoned' is never seen here. Ask instead of assuming.
  const hidden = ref(!(await win.isVisible()))
  const keepOpen = ref(false)
  // The window-state plugin restores the maximised state, so the window may start in it.
  const maximized = ref(await win.isMaximized())
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

  // Maximising by the double click on the header or by Win+Up also resizes the window,
  // so the resize event is where the state is learnt. Events that arrive while the
  // question is out are folded into one more question.
  let asking = false
  let askAgain = false
  async function syncMaximized() {
    if (asking) {
      askAgain = true
      return
    }
    asking = true
    try {
      do {
        askAgain = false
        maximized.value = await win.isMaximized()
      } while (askAgain)
    } finally {
      asking = false
    }
  }

  // The listeners live as long as the page, so their unlisten functions are not kept.
  await listen(EVENT_SUMMONED, markSummoned)
  await listen(EVENT_HIDDEN, markHidden)
  await win.onResized(() => void syncMaximized())
  await win.onFocusChanged(({ payload: focused }) => {
    if (!focused && hidesOnBlur()) void hide()
  })

  return {
    kind: 'tauri',
    bridge: createTauriBridge(),
    hidden: readonly(hidden),
    keepOpen,

    async show() {
      await win.show()
      await win.setFocus()
      markSummoned()
    },

    hide,

    maximized: readonly(maximized),

    async toggleMaximize() {
      await win.toggleMaximize()
      await syncMaximized()
    },

    async workArea(): Promise<Size | null> {
      const monitor = await currentMonitor()
      if (!monitor) return null
      // The work area is in physical pixels; the layout is measured in logical ones.
      const { width, height } = monitor.workArea.size
      return { width: Math.floor(width / monitor.scaleFactor), height: Math.floor(height / monitor.scaleFactor) }
    },

    async applyZoom(percent) {
      const factor = percent / 100
      await getCurrentWebview().setZoom(factor)
      const min = new LogicalSize(Math.ceil(MIN_LAYOUT.width * factor), Math.ceil(MIN_LAYOUT.height * factor))
      await win.setMinSize(min)
      // A maximised window already fills the work area, which the zoom levels were chosen to fit.
      if (await win.isMaximized()) return
      const size = (await win.innerSize()).toLogical(await win.scaleFactor())
      if (size.width < min.width || size.height < min.height) {
        await win.setSize(new LogicalSize(Math.max(size.width, min.width), Math.max(size.height, min.height)))
      }
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
