// How a habit's value becomes a heatmap cell, a streak and a completion rate.
// Pure functions over the api's habit days; nothing here reads or writes state.
import type { Habit, HabitDay, ISODate } from '@/data/types'
import { addDays } from './dates'

export type CellState = 'none' | 'miss' | 'skip' | 'heat-1' | 'heat-2' | 'heat-3' | 'heat-4'

const STEPS = ['heat-1', 'heat-2', 'heat-3', 'heat-4'] as const

/** The heat step for a share of the target, 0 < share <= 1, in four equal steps. */
function stepOf(share: number): CellState {
  return STEPS[Math.min(STEPS.length - 1, Math.max(0, Math.ceil(share * STEPS.length) - 1))]
}

function targetOf(habit: Habit): number {
  return habit.target && habit.target > 0 ? habit.target : 1
}

/** How one habit looks on one day. */
export function cellState(habit: Habit, day: HabitDay | undefined): CellState {
  if (!day?.noteExists) return 'none'
  const value = day.values[habit.id]
  if (value === 'skip') return 'skip'
  if (value === true) return 'heat-3'
  if (typeof value === 'number' && value > 0) return stepOf(value / targetOf(habit))
  return 'miss'
}

/** How much of the habit was done, 0..1; null when the day does not count (no note, or skipped). */
function shareOf(habit: Habit, day: HabitDay | undefined): number | null {
  if (!day?.noteExists) return null
  const value = day.values[habit.id]
  if (value === 'skip') return null
  if (value === true) return 1
  if (typeof value === 'number') return Math.min(1, value / targetOf(habit))
  return 0
}

function isDone(habit: Habit, day: HabitDay | undefined): boolean {
  return shareOf(habit, day) === 1
}

/** The "All" cell of the year view: the mean share of every habit that counts that day. */
export function allState(habits: Habit[], day: HabitDay | undefined): CellState {
  if (!day?.noteExists) return 'none'
  const shares = habits.flatMap((habit) => shareOf(habit, day) ?? [])
  if (shares.length === 0) return 'skip'
  const mean = shares.reduce((sum, share) => sum + share, 0) / shares.length
  return mean > 0 ? stepOf(mean) : 'miss'
}

/** How many habits were done and how many counted on a day, for the "All" read-out. */
export function doneCount(habits: Habit[], day: HabitDay | undefined): { done: number; counted: number } {
  const counted = habits.filter((habit) => shareOf(habit, day) !== null)
  return { done: counted.filter((habit) => isDone(habit, day)).length, counted: counted.length }
}

/**
 * Days in a row, ending on `date`, on which the habit was done. A skipped day
 * neither adds nor breaks the run. Today still counts as open when it is not done yet.
 */
export function streakAt(habit: Habit, days: Map<ISODate, HabitDay>, date: ISODate, today: ISODate): number {
  let cursor = date
  if (cursor === today && !isDone(habit, days.get(cursor))) cursor = addDays(cursor, -1)

  let streak = 0
  for (let day = days.get(cursor); day; day = days.get(cursor)) {
    if (day.noteExists && day.values[habit.id] === 'skip') {
      cursor = addDays(cursor, -1)
      continue
    }
    if (!isDone(habit, day)) break
    streak += 1
    cursor = addDays(cursor, -1)
  }
  return streak
}

/** Share of counted days on which the habit was done, 0..100; null when no day counted. */
export function completionPercent(habit: Habit, dayList: HabitDay[]): number | null {
  const counted = dayList.filter((day) => shareOf(habit, day) !== null)
  if (counted.length === 0) return null
  return Math.round((counted.filter((day) => isDone(habit, day)).length / counted.length) * 100)
}

/** One habit on one day in words, for the read-out: "Reading 30 min", "Exercise done". */
export function habitWords(habit: Habit, day: HabitDay | undefined): string {
  const value = day?.noteExists ? day.values[habit.id] : undefined
  if (value === 'skip') return `${habit.name} skipped`
  if (value === true) return `${habit.name} done`
  if (typeof value === 'number') return `${habit.name} ${value}${habit.unit ? ` ${habit.unit}` : ''}`
  return `${habit.name} not done`
}
