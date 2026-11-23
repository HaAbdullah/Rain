// Types that only the Life screen and its actions share.
import type { Habit, ISODate } from '@/data/types'

/** One day of the open habit, the target of Enter and Ctrl+K in the full view. */
export interface HabitDayItem {
  habit: Habit
  date: ISODate
  /** The note of that day has the property set to true. */
  done: boolean
  noteExists: boolean
  /** Rain may change that day: it is one of the last 31 days. Older days are changed in Obsidian. */
  tickable: boolean
}
