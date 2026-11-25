// Zoom of the whole page: Ctrl+plus, Ctrl+minus and Ctrl+0 (spec 4.2), and the
// Settings row that other code builds on this interface. The level is remembered
// and applied before the app is set up, because this module waits for it.
//
// The step logic is in zoom-steps.ts. The shell does the real work: the WebView's
// own zoom in Tauri, CSS zoom on the root element in a browser tab.
import { readonly, ref } from 'vue'
import { shell } from '@/shell'
import { clampZoom, DEFAULT_ZOOM, snapToStep, stepZoom } from './zoom-steps'

const STORAGE_KEY = 'rain.zoom'
const PILL_MS = 1000

const level = ref(DEFAULT_ZOOM)

/** The zoom in percent. */
export const zoom = readonly(level)

function remembered(): number {
  try {
    const stored = Number(localStorage.getItem(STORAGE_KEY))
    return stored > 0 ? snapToStep(stored) : DEFAULT_ZOOM
  } catch {
    return DEFAULT_ZOOM
  }
}

function remember(percent: number) {
  try {
    localStorage.setItem(STORAGE_KEY, String(percent))
  } catch {
    // Without storage the level lasts until the next start.
  }
}

// ---- the pill ----

let pill: HTMLElement | null = null
let pillTimer: number | undefined

/** Shows "125%" for a second. A plain element, so no screen has to mount it. */
function showPill(percent: number) {
  if (typeof document === 'undefined') return
  if (!pill) {
    pill = document.createElement('div')
    pill.setAttribute('role', 'status')
    pill.className =
      'pointer-events-none fixed bottom-6 left-1/2 z-[100] -translate-x-1/2 rounded-full border border-border-strong bg-surface-2 px-3 py-1 text-13 font-medium text-foreground shadow-lg transition-opacity duration-150'
    document.body.append(pill)
  }
  pill.textContent = `${percent}%`
  pill.style.opacity = '1'
  window.clearTimeout(pillTimer)
  pillTimer = window.setTimeout(() => {
    if (pill) pill.style.opacity = '0'
  }, PILL_MS)
}

// ---- changes ----

// Changes run one after the other: a held key must not start a second window resize
// before the first one has finished.
let queue: Promise<void> = Promise.resolve()

async function change(next: (current: number, area: Awaited<ReturnType<typeof shell.workArea>>) => number, user: boolean) {
  const area = await shell.workArea()
  const percent = clampZoom(next(level.value, area), area)
  await shell.applyZoom(percent)
  level.value = percent
  if (user) {
    remember(percent)
    showPill(percent)
  }
}

function enqueue(next: Parameters<typeof change>[0], user = true): Promise<void> {
  const run = queue.then(() => change(next, user))
  // A failed change must not block the next one.
  queue = run.catch(() => {})
  return run
}

export function zoomIn(): Promise<void> {
  return enqueue((current, area) => stepZoom(current, 1, area))
}

export function zoomOut(): Promise<void> {
  return enqueue((current, area) => stepZoom(current, -1, area))
}

export function resetZoom(): Promise<void> {
  return enqueue(() => DEFAULT_ZOOM)
}

/** Sets a level. A value between two levels, or one that does not fit the monitor, becomes the nearest level that does. */
export function setZoom(percent: number): Promise<void> {
  return enqueue(() => percent)
}

export function useZoom() {
  return { zoom, zoomIn, zoomOut, resetZoom, setZoom }
}

// Applied once, at start: importing modules wait for this line, like they wait for the shell.
try {
  await enqueue(() => remembered(), false)
} catch {
  // The window keeps its normal size; the keys still work.
}
