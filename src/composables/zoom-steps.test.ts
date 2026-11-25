import { describe, expect, it } from 'vitest'
import { allowedSteps, clampZoom, fitsArea, minWindowSize, snapToStep, stepZoom, ZOOM_STEPS } from './zoom-steps'

// The owner's monitor: 2560 x 1440 at 100% scale, minus a taskbar.
const BIG = { width: 2560, height: 1392 }
// A laptop: 1920 x 1080 at 150% scale is 1280 x 720 logical, minus a taskbar.
const SMALL = { width: 1280, height: 720 }

describe('zoom steps', () => {
  it('offers the eight levels', () => {
    expect(ZOOM_STEPS).toEqual([80, 90, 100, 110, 125, 150, 175, 200])
  })

  it('walks up and down one step and stops at the ends', () => {
    expect(stepZoom(100, 1, null)).toBe(110)
    expect(stepZoom(110, 1, null)).toBe(125)
    expect(stepZoom(100, -1, null)).toBe(90)
    expect(stepZoom(80, -1, null)).toBe(80)
    expect(stepZoom(200, 1, null)).toBe(200)
  })

  it('snaps a value between two levels to the nearest, and a tie to the smaller', () => {
    expect(snapToStep(104)).toBe(100)
    expect(snapToStep(106)).toBe(110)
    expect(snapToStep(105)).toBe(100)
    expect(snapToStep(Number.NaN)).toBe(100)
    expect(stepZoom(104, 1, null)).toBe(110)
  })
})

describe('zoom and the monitor', () => {
  it('the minimum window grows with the zoom', () => {
    expect(minWindowSize(100)).toEqual({ width: 1040, height: 680 })
    expect(minWindowSize(125)).toEqual({ width: 1300, height: 850 })
    expect(minWindowSize(200)).toEqual({ width: 2080, height: 1360 })
  })

  it('every level fits the 32 inch monitor', () => {
    expect(allowedSteps(BIG)).toEqual([...ZOOM_STEPS])
  })

  it('a level whose minimum window does not fit is not offered', () => {
    expect(fitsArea(125, SMALL)).toBe(false)
    expect(allowedSteps(SMALL)).toEqual([80, 90, 100])
    expect(stepZoom(100, 1, SMALL)).toBe(100)
  })

  it('a saved level that no longer fits is pulled down to the largest that does', () => {
    expect(clampZoom(200, SMALL)).toBe(100)
    expect(clampZoom(200, BIG)).toBe(200)
    expect(clampZoom(200, null)).toBe(200)
  })

  it('keeps the smallest level when nothing fits', () => {
    expect(allowedSteps({ width: 400, height: 300 })).toEqual([80])
    expect(clampZoom(150, { width: 400, height: 300 })).toBe(80)
  })
})
