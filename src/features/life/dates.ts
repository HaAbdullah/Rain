// Calendar arithmetic on YYYY-MM-DD strings, in local time. Every date is built
// from its parts: new Date('2026-11-03') would be read as UTC (the day before,
// west of Greenwich) and toISOString() would give the UTC date back.
import type { ISODate } from '@/data/types'
import { toISODate } from '@/lib/time'

const MS_PER_DAY = 24 * 60 * 60 * 1000

export function parseISODate(iso: ISODate): Date {
  const [year, month, day] = iso.split('-').map(Number)
  return new Date(year, month - 1, day)
}

export function addDays(iso: ISODate, days: number): ISODate {
  const d = parseISODate(iso)
  d.setDate(d.getDate() + days)
  return toISODate(d)
}

export function daysInMonth(year: number, monthIndex: number): number {
  return new Date(year, monthIndex + 1, 0).getDate()
}

/** Moves by whole months and keeps the day of the month, or the last day when the month is shorter. */
export function addMonths(iso: ISODate, months: number): ISODate {
  const d = parseISODate(iso)
  const first = new Date(d.getFullYear(), d.getMonth() + months, 1)
  first.setDate(Math.min(d.getDate(), daysInMonth(first.getFullYear(), first.getMonth())))
  return toISODate(first)
}

/** Days from `from` to `to` (negative when `to` is earlier). Counted in UTC so a DST change cannot skew it. */
export function daysBetween(from: ISODate, to: ISODate): number {
  const a = parseISODate(from)
  const b = parseISODate(to)
  const utc = (d: Date) => Date.UTC(d.getFullYear(), d.getMonth(), d.getDate())
  return Math.round((utc(b) - utc(a)) / MS_PER_DAY)
}

/** Monday is 0, Sunday is 6: the row order of the year grid. */
export function weekdayRow(iso: ISODate): number {
  return (parseISODate(iso).getDay() + 6) % 7
}

export function isWeekend(iso: ISODate): boolean {
  const day = parseISODate(iso).getDay()
  return day === 0 || day === 6
}

export function clampDate(iso: ISODate, min: ISODate, max: ISODate): ISODate {
  // ISO dates compare correctly as text.
  if (iso < min) return min
  return iso > max ? max : iso
}
