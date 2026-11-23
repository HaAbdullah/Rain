// Where each day of a year sits in the year grid: one column per week (Monday to Sunday),
// one row per weekday. Pure, so that it can be tested; HabitYear.vue only draws it.
import type { ISODate } from '@/data/types'
import { toISODate } from '@/lib/time'
import { daysBetween, weekdayRow } from './dates'

export interface YearCell {
  date: ISODate
  /** 0 is the week of 1 January. */
  column: number
  /** 0 is Monday, 6 is Sunday. */
  row: number
}

export interface MonthLabel {
  name: string
  column: number
}

export interface YearGrid {
  cells: YearCell[]
  labels: MonthLabel[]
  /** How many week columns the grid has. */
  weekCount: number
}

// Fixed, not from Intl: some locales write "Sept", and the labels must fit over one column.
const MONTH_NAMES = ['Jan', 'Feb', 'Mar', 'Apr', 'May', 'Jun', 'Jul', 'Aug', 'Sep', 'Oct', 'Nov', 'Dec']

/**
 * The grid of `year`. Days after `today` are not part of it: the future is not drawn. A month
 * label sits over the column of the 1st and only exists once that day has come.
 */
export function yearGrid(year: number, today: ISODate): YearGrid {
  const first = toISODate(new Date(year, 0, 1))
  const length = daysBetween(first, toISODate(new Date(year, 11, 31))) + 1
  // Grid slots before 1 January, so that the first week lines up under Monday.
  const lead = weekdayRow(first)
  const columnOf = (date: ISODate) => Math.floor((daysBetween(first, date) + lead) / 7)

  const drawn = Math.max(0, Math.min(length, daysBetween(first, today) + 1))
  const cells = Array.from({ length: drawn }, (_, i) => {
    const date = toISODate(new Date(year, 0, i + 1))
    return { date, column: columnOf(date), row: weekdayRow(date) }
  })

  const labels = MONTH_NAMES.flatMap((name, month) => {
    const date = toISODate(new Date(year, month, 1))
    return date <= today ? [{ name, column: columnOf(date) }] : []
  })

  return { cells, labels, weekCount: Math.ceil((lead + length) / 7) }
}
