import { beforeEach, describe, expect, it, vi } from 'vitest'

// The window API only exists inside the window, so it is replaced by a small fake.
const windowState = { maximized: false, resize: () => {} }
const fakeWindow = {
  isVisible: vi.fn(async () => true),
  isMaximized: vi.fn(async () => windowState.maximized),
  toggleMaximize: vi.fn(async () => {
    windowState.maximized = !windowState.maximized
  }),
  onFocusChanged: vi.fn(async () => () => {}),
  onResized: vi.fn(async (cb: () => void) => {
    windowState.resize = cb
    return () => {}
  }),
  setMinSize: vi.fn(async () => {}),
  setSize: vi.fn(async () => {}),
  innerSize: vi.fn(async () => ({ toLogical: () => ({ width: 1280, height: 800 }) })),
  scaleFactor: vi.fn(async () => 1),
}
const currentMonitor = vi.fn(async () => ({ workArea: { size: { width: 2560, height: 1392 } }, scaleFactor: 1 }))
const setZoom = vi.fn(async () => {})

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }))
vi.mock('@tauri-apps/api/event', () => ({ listen: () => Promise.resolve(() => {}) }))
vi.mock('@tauri-apps/api/webview', () => ({ getCurrentWebview: () => ({ setZoom }) }))
vi.mock('@tauri-apps/api/window', () => ({
  getCurrentWindow: () => fakeWindow,
  currentMonitor: () => currentMonitor(),
  LogicalSize: class {
    width: number
    height: number
    constructor(width: number, height: number) {
      this.width = width
      this.height = height
    }
  },
}))

import { createTauriShell } from './tauri'

beforeEach(() => {
  windowState.maximized = false
  vi.clearAllMocks()
  vi.stubGlobal('document', { createElement: () => ({}), head: { append: () => {} } })
})

describe('tauri shell: maximise', () => {
  it('starts with the state of the real window', async () => {
    windowState.maximized = true
    expect((await createTauriShell()).maximized.value).toBe(true)
  })

  it('maximises and restores through the window, and follows the result', async () => {
    const shell = await createTauriShell()
    await shell.toggleMaximize()
    expect(shell.maximized.value).toBe(true)
    await shell.toggleMaximize()
    expect(shell.maximized.value).toBe(false)
  })

  it('follows a maximise by double click or Win+Up, which arrives as a resize event', async () => {
    const shell = await createTauriShell()
    windowState.maximized = true
    windowState.resize()
    await vi.waitFor(() => expect(shell.maximized.value).toBe(true))
    windowState.maximized = false
    windowState.resize()
    await vi.waitFor(() => expect(shell.maximized.value).toBe(false))
  })
})

describe('tauri shell: zoom', () => {
  it('sets the WebView zoom and a minimum size of 1040 x 680 times the zoom', async () => {
    const shell = await createTauriShell()
    await shell.applyZoom(150)
    expect(setZoom).toHaveBeenCalledWith(1.5)
    expect(fakeWindow.setMinSize).toHaveBeenCalledWith(expect.objectContaining({ width: 1560, height: 1020 }))
  })

  it('grows a window that is smaller than the new minimum, and leaves a bigger one alone', async () => {
    const shell = await createTauriShell()
    await shell.applyZoom(150)
    expect(fakeWindow.setSize).toHaveBeenCalledWith(expect.objectContaining({ width: 1560, height: 1020 }))
    fakeWindow.setSize.mockClear()
    await shell.applyZoom(100)
    expect(fakeWindow.setSize).not.toHaveBeenCalled()
  })

  it('does not resize a maximised window', async () => {
    windowState.maximized = true
    const shell = await createTauriShell()
    await shell.applyZoom(200)
    expect(fakeWindow.setSize).not.toHaveBeenCalled()
  })

  it('reports the work area in logical pixels', async () => {
    currentMonitor.mockResolvedValueOnce({
      workArea: { size: { width: 3840, height: 2080 } },
      scaleFactor: 1.5,
    })
    expect(await (await createTauriShell()).workArea()).toEqual({ width: 2560, height: 1386 })
  })
})
