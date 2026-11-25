// The aggregate heatmap at the bottom of Life: one cell per day, how much of that day's habits
// were done. Pure, so that it can be tested; AllHabitsHeat.vue only draws it.
import type { HabitDay, HabitSummary, ISODate } from '@/data/types'
import { toISODate } from '@/lib/time'
import { addDays, parseISODate, weekdayRow } from './dates'

/** The most week columns the map shows: a year and the part week at its start. */
export const HEAT_MAX_WEEKS = 53

export type HeatStep = 0 | 1 | 2 | 3 | 4

export interface HeatDay {
  date: ISODate
  /** Week column, 0 is the oldest. */
  column: number
  /** 0 is Monday, 6 is Sunday. */
  row: number
  /** Habits ticked that day, capped at `total` so that the tooltip never says "20 of 18". */
  done: number
  /** The daily habits (or all habits when there is no daily one). */
  total: number
  /** 0 to 1. */
  value: number
  step: HeatStep
}

export interface HeatLabel {
  name: string
  column: number
}

export interface HeatModel {
  days: HeatDay[]
  labels: HeatLabel[]
  weekCount: number
  /** Days where everything was done. */
  fullDays: number
}

const MONTH_NAMES = ['Jan', 'Feb', 'Mar', 'Apr', 'May', 'Jun', 'Jul', 'Aug', 'Sep', 'Oct', 'Nov', 'Dec']

function clampWeeks(weeks: number): number {
  return Math.max(1, Math.min(HEAT_MAX_WEEKS, Math.floor(weeks)))
}

/** The Monday of the first week the map shows, `weeks` columns ending with the week of `today`. */
export function heatStart(today: ISODate, weeks: number): ISODate {
  return addDays(today, -weekdayRow(today) - 7 * (clampWeeks(weeks) - 1))
}

/**
 * The five steps of a value: 0 is nothing done, 4 is more than three quarters. Any tick lifts a
 * day off step 0.
 */
export function heatStep(value: number): HeatStep {
  if (!(value > 0)) return 0
  return Math.min(4, Math.max(1, Math.ceil(value * 4))) as HeatStep
}

/**
 * Builds the map. The value of a day is the habits ticked that day (any cadence) divided by the
 * number of daily habits, capped at 1. With no daily habit the divisor is the number of habits.
 * Ticks of habits that no longer exist are ignored. Days without a note count as 0, days after
 * `today` are not drawn. Today's ticks come from the list rows, which change the moment a box
 * is ticked.
 */
export function heatModel(
  habits: readonly HabitSummary[],
  days: readonly HabitDay[],
  today: ISODate,
  weeks: number,
): HeatModel {
  const columns = clampWeeks(weeks)
  const start = heatStart(today, columns)
  const known = new Set(habits.map((summary) => summary.habit.id))
  const daily = habits.filter((summary) => summary.habit.cadence === 'daily').length
  const total = daily > 0 ? daily : habits.length

  const ticks = new Map<ISODate, number>()
  for (const day of days) {
    ticks.set(day.date, new Set(day.done.filter((id) => known.has(id))).size)
  }
  ticks.set(today, habits.filter((summary) => summary.todayDone).length)

  const cells: HeatDay[] = []
  let fullDays = 0
  const first = parseISODate(start)
  for (let offset = 0; ; offset++) {
    const date = toISODate(new Date(first.getFullYear(), first.getMonth(), first.getDate() + offset))
    if (date > today) break
    const count = ticks.get(date) ?? 0
    const value = total > 0 ? Math.min(1, count / total) : 0
    if (total > 0 && value === 1) fullDays += 1
    cells.push({
      date,
      column: Math.floor(offset / 7),
      row: offset % 7,
      done: Math.min(count, total),
      total,
      value,
      step: heatStep(value),
    })
  }

  return { days: cells, labels: monthLabels(start, columns), weekCount: columns, fullDays }
}

/** A name over the first week column of each month; one that would touch the next is dropped. */
function monthLabels(start: ISODate, columns: number): HeatLabel[] {
  const labels: HeatLabel[] = []
  let previous = -1
  for (let column = 0; column < columns; column++) {
    const month = parseISODate(addDays(start, column * 7)).getMonth()
    if (month === previous) continue
    previous = month
    const last = labels[labels.length - 1]
    if (last && column - last.column < 3) labels.pop()
    labels.push({ name: MONTH_NAMES[month], column })
  }
  return labels
}

/** How many week columns fit in `width` px of cells of `cell` px with `gap` px between. */
export function weeksThatFit(width: number, cell: number, gap: number): number {
  return clampWeeks(Math.floor((width + gap) / (cell + gap)))
}
