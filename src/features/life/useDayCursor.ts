// The day cursor of the Life screen: custom 2D navigation over days and habits.
// The cursor starts on today and never leaves the range that is loaded. Its state
// lives in useViewState, so it survives a short hide of the window.
import { computed, type ComputedRef, type Ref } from 'vue'
import type { Dir } from '@/composables/useScreen'
import { useViewState } from '@/composables/useViewState'
import type { ID, ISODate } from '@/data/types'
import { addDays, addMonths, clampDate } from './dates'

export type LifeView = 'month' | 'year'

/** The chip of the year view that stands for every habit. */
export const ALL_HABITS = 'all'

interface CursorState {
  /** '' means today, so a cursor that was never moved follows the calendar past midnight. */
  date: ISODate
  /** The habit row of the month view; '' means the first row. */
  row: ID
  /** The habit chip of the year view. */
  chip: ID
}

export interface DayCursor {
  date: ComputedRef<ISODate>
  row: ComputedRef<ID | null>
  chip: ComputedRef<ID>
  move(dir: Dir): boolean
  /** PageUp / PageDown and the period buttons: a month in Month, a year in Year. */
  page(dir: -1 | 1): void
  /** Ctrl+Up / Ctrl+Down: the previous or next habit chip (Year only). */
  group(dir: -1 | 1): void
  select(date: ISODate, row?: ID): void
  selectChip(id: ID): void
  goToday(): void
}

interface CursorOptions {
  view: Ref<LifeView>
  today: Ref<ISODate>
  /** The oldest day that is loaded. */
  min: Ref<ISODate>
  /** Habit ids of the month rows now shown. */
  rows: Ref<ID[]>
  /** Chip ids of the year view, "all" first. */
  chips: Ref<ID[]>
}

/** The id `step` places away from `current` inside `ids`, stopping at the ends. */
function stepIn(ids: ID[], current: ID | null, step: number): ID | undefined {
  const index = current === null ? 0 : ids.indexOf(current)
  return ids[Math.min(ids.length - 1, Math.max(0, index + step))]
}

export function useDayCursor({ view, today, min, rows, chips }: CursorOptions): DayCursor {
  const state = useViewState<CursorState>('life.cursor', { date: '', row: '', chip: ALL_HABITS })

  const date = computed(() => clampDate(state.date || today.value, min.value, today.value))
  const row = computed(() => (rows.value.includes(state.row) ? state.row : (rows.value[0] ?? null)))
  const chip = computed(() => (chips.value.includes(state.chip) ? state.chip : ALL_HABITS))

  function setDate(next: ISODate) {
    state.date = clampDate(next, min.value, today.value)
  }

  return {
    date,
    row,
    chip,

    // Month: left/right change the day (and roll into the next month), up/down the habit.
    // Year: the grid runs in columns of weeks, so left/right change the week, up/down the day.
    move(dir) {
      const sign = dir === 'left' || dir === 'up' ? -1 : 1
      const sideways = dir === 'left' || dir === 'right'
      if (view.value === 'year') setDate(addDays(date.value, sideways ? sign * 7 : sign))
      else if (sideways) setDate(addDays(date.value, sign))
      else state.row = stepIn(rows.value, row.value, sign) ?? ''
      return true
    },

    page(dir) {
      setDate(addMonths(date.value, view.value === 'year' ? dir * 12 : dir))
    },

    group(dir) {
      if (view.value === 'year') state.chip = stepIn(chips.value, chip.value, dir) ?? ALL_HABITS
    },

    select(next, habit) {
      setDate(next)
      if (habit !== undefined) state.row = habit
    },

    selectChip(id) {
      state.chip = id
    },

    goToday() {
      state.date = ''
    },
  }
}
