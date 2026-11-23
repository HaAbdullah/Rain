import { describe, expect, it } from 'vitest'
import { addDays, addMonths, clampDate, daysBetween, weekdayRow } from './dates'

describe('addDays', () => {
  it('crosses month and year ends', () => {
    expect(addDays('2026-11-22', 1)).toBe('2026-11-23')
    expect(addDays('2026-12-31', 1)).toBe('2027-01-01')
    expect(addDays('2027-01-01', -1)).toBe('2026-12-31')
  })

  it('knows the leap day', () => {
    expect(addDays('2024-02-28', 1)).toBe('2024-02-29')
    expect(addDays('2026-02-28', 1)).toBe('2026-03-01')
  })
})

describe('addMonths', () => {
  it('keeps the day of the month', () => {
    expect(addMonths('2026-11-22', 1)).toBe('2026-12-22')
    expect(addMonths('2026-11-22', -12)).toBe('2025-11-22')
  })

  it('clamps to the last day of a shorter month', () => {
    expect(addMonths('2026-01-31', 1)).toBe('2026-02-28')
    expect(addMonths('2024-01-31', 1)).toBe('2024-02-29')
    expect(addMonths('2026-03-31', -1)).toBe('2026-02-28')
  })
})

describe('daysBetween', () => {
  it('counts whole days, also across a daylight saving change', () => {
    expect(daysBetween('2026-11-22', '2026-11-22')).toBe(0)
    expect(daysBetween('2026-11-22', '2026-11-29')).toBe(7)
    expect(daysBetween('2026-11-29', '2026-11-22')).toBe(-7)
    expect(daysBetween('2026-03-01', '2026-04-01')).toBe(31) // a clock change in most zones
    expect(daysBetween('2026-10-01', '2026-11-30')).toBe(60)
  })
})

describe('weekdayRow', () => {
  it('is 0 on Monday and 6 on Sunday', () => {
    expect(weekdayRow('2026-11-16')).toBe(0)
    expect(weekdayRow('2026-11-22')).toBe(6)
    expect(weekdayRow('2026-01-01')).toBe(3)
  })
})

describe('clampDate', () => {
  it('keeps a date inside the range', () => {
    expect(clampDate('2026-11-10', '2026-11-01', '2026-11-22')).toBe('2026-11-10')
    expect(clampDate('2026-10-01', '2026-11-01', '2026-11-22')).toBe('2026-11-01')
    expect(clampDate('2026-12-01', '2026-11-01', '2026-11-22')).toBe('2026-11-22')
  })
})
