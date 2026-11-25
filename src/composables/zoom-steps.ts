// The pure part of zoom: which levels exist, and which of them fit a monitor.
// useZoom.ts adds the state, the storage and the window.
import { MIN_LAYOUT, type Size } from '@/shell/shell'

export const ZOOM_STEPS: readonly number[] = [80, 90, 100, 110, 125, 150, 175, 200]
export const DEFAULT_ZOOM = 100

/** The smallest window, in logical pixels, in which the layout still fits at this zoom. */
export function minWindowSize(percent: number): Size {
  return {
    width: Math.ceil((MIN_LAYOUT.width * percent) / 100),
    height: Math.ceil((MIN_LAYOUT.height * percent) / 100),
  }
}

/** True when a window of the minimum size for this zoom fits the work area. An unknown area fits everything. */
export function fitsArea(percent: number, area: Size | null): boolean {
  if (area === null) return true
  const min = minWindowSize(percent)
  return min.width <= area.width && min.height <= area.height
}

/** The levels on offer: those that fit. The smallest level is always kept, so the list is never empty. */
export function allowedSteps(area: Size | null): number[] {
  const fitting = ZOOM_STEPS.filter((step) => fitsArea(step, area))
  return fitting.length > 0 ? fitting : [ZOOM_STEPS[0]]
}

/** The step nearest to a value; a tie goes to the smaller one. A value that is not a number is 100. */
export function snapToStep(percent: number, steps: readonly number[] = ZOOM_STEPS): number {
  if (!Number.isFinite(percent)) return DEFAULT_ZOOM
  return steps.reduce((best, step) => (Math.abs(step - percent) < Math.abs(best - percent) ? step : best))
}

/** A value brought to a level that exists and fits the monitor. */
export function clampZoom(percent: number, area: Size | null): number {
  return snapToStep(percent, allowedSteps(area))
}

/** One step up or down from a level. At either end it stays where it is. */
export function stepZoom(current: number, direction: 1 | -1, area: Size | null): number {
  const steps = allowedSteps(area)
  const here = snapToStep(current, steps)
  const next = steps.indexOf(here) + direction
  return steps[Math.min(Math.max(next, 0), steps.length - 1)]
}
