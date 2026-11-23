// Everything the full view of one habit needs: its stats, its days of the shown year, and the
// cursor that moves over the year grid. It lives in a composable (not in the view component)
// because the Life screen hands the cursor's keys and the selected day to the shell.
//
// The backend computes streaks, rate and totals; this file only fetches them and moves the cursor.
import { computed, watch, type ComputedRef, type Ref } from 'vue'
import { useResource } from '@/composables/useResource'
import type { Dir } from '@/composables/useScreen'
import { api } from '@/data'
import type { HabitCell, HabitDay, HabitStats, HabitSummary, ISODate } from '@/data/types'
import { formatDayLabel } from '@/lib/time'
import { lifeCursor, lifeView } from './actions'
import { addDays, addMonths, clampDate } from './dates'
import type { HabitDayItem } from './types'

/** The oldest day the backend reads, until the stats say it (`HabitStats.windowFrom`): 800 days. */
const WINDOW_DAYS = 800
/** The oldest day Rain changes, until the stats say it: today and the 30 days before. */
const TICK_DAYS = 31

/** The days of one habit in one year, remembered with what they belong to. */
interface YearDays {
  habitId: string
  year: number
  days: HabitDay[]
}

export interface OpenHabit {
  /** The summary of the open habit, undefined when none is open or it no longer exists. */
  habit: ComputedRef<HabitSummary | undefined>
  stats: Readonly<Ref<HabitStats | null | undefined>>
  /** True once the stats and the days of the shown year belong to the open habit. */
  ready: ComputedRef<boolean>
  failed: ComputedRef<boolean>
  reload(): void
  year: ComputedRef<number>
  /** The day the cursor is on. */
  date: ComputedRef<ISODate>
  cellState(date: ISODate): HabitCell
  /** The selected day as the target of Enter and Ctrl+K. */
  dayItem: ComputedRef<HabitDayItem | null>
  /** "Sun 22 Nov: done" */
  readout: ComputedRef<string>
  windowFrom: ComputedRef<ISODate>
  tickableFrom: ComputedRef<ISODate>
  move(dir: Dir): boolean
  /** PageUp / PageDown and the year buttons: one year. */
  page(dir: -1 | 1): void
  select(date: ISODate): void
}

export function useOpenHabit(habits: Ref<HabitSummary[]>, today: Ref<ISODate>): OpenHabit {
  const habit = computed(() => habits.value.find((summary) => summary.habit.id === lifeView.habitId))

  // ---- the cursor ----

  const stats = useResource<HabitStats | null>('habitStats:open', () =>
    lifeView.habitId ? api.getHabitStats(lifeView.habitId) : Promise.resolve(null),
  )
  const windowFrom = computed(() => stats.data.value?.windowFrom ?? addDays(today.value, -(WINDOW_DAYS - 1)))
  const tickableFrom = computed(() => stats.data.value?.tickableFrom ?? addDays(today.value, -(TICK_DAYS - 1)))

  const date = computed(() => clampDate(lifeCursor.date || today.value, windowFrom.value, today.value))
  const year = computed(() => Number(date.value.slice(0, 4)))

  function setDate(next: ISODate) {
    lifeCursor.date = clampDate(next, windowFrom.value, today.value)
  }

  // ---- the days of the shown year ----

  const yearDays = useResource<YearDays | null>('habitDays:open', async () => {
    const habitId = lifeView.habitId
    if (!habitId) return null
    const shown = year.value
    const last = `${shown}-12-31`
    const days = await api.listHabitDays(`${shown}-01-01`, last < today.value ? last : today.value, habitId)
    return { habitId, year: shown, days }
  })

  const rows = computed(() => new Map((yearDays.data.value?.days ?? []).map((day) => [day.date, day])))

  function reload() {
    void stats.reload()
    void yearDays.reload()
  }

  // A different habit, another year, a new day, or a change of the habit itself (a tick, an edit
  // in Obsidian that the `habits` topic carried): fetch again.
  watch([() => lifeView.habitId, year, today, () => JSON.stringify(habit.value)], reload)

  const ready = computed(
    () =>
      habit.value !== undefined &&
      stats.data.value?.habitId === habit.value.habit.id &&
      yearDays.data.value?.habitId === habit.value.habit.id &&
      yearDays.data.value.year === year.value,
  )

  // ---- what the grid and the shell read ----

  function cellState(day: ISODate): HabitCell {
    const row = rows.value.get(day)
    if (!row) return 'none'
    return habit.value && row.done.includes(habit.value.habit.id) ? 'done' : 'miss'
  }

  const dayItem = computed<HabitDayItem | null>(() => {
    const open = habit.value
    if (!open) return null
    const day = date.value
    return {
      habit: open.habit,
      date: day,
      done: cellState(day) === 'done',
      noteExists: rows.value.has(day),
      tickable: day >= tickableFrom.value && day <= today.value,
    }
  })

  const readout = computed(() => {
    const item = dayItem.value
    if (!item) return ''
    const state = !item.noteExists ? 'no note' : item.done ? 'done' : 'not done'
    return `${formatDayLabel(item.date)}: ${state}`
  })

  return {
    habit,
    stats: stats.data,
    ready,
    failed: computed(() => stats.error.value !== null || yearDays.error.value !== null),
    reload,
    year,
    date,
    cellState,
    dayItem,
    readout,
    windowFrom,
    tickableFrom,

    // The grid runs in columns of weeks: left and right change the week, up and down the day.
    move(dir) {
      const sign = dir === 'left' || dir === 'up' ? -1 : 1
      const sideways = dir === 'left' || dir === 'right'
      setDate(addDays(date.value, sideways ? sign * 7 : sign))
      return true
    },

    page(dir) {
      setDate(addMonths(date.value, dir * 12))
    },

    select: setDate,
  }
}
