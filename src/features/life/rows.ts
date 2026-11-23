// The rows of the Life list: habits grouped by how often they are meant to be done.
import type { SelGroup } from '@/composables/useSelection'
import type { Cadence, HabitSummary } from '@/data/types'
import { matches } from '@/lib/score'

/** A summary with the `id` that selection and lists need: the id of the habit. */
export type HabitRow = HabitSummary & { id: string }

const GROUPS: { id: Cadence; title: string }[] = [
  { id: 'daily', title: 'Daily' },
  { id: 'weekly', title: 'Weekly' },
  { id: 'monthly', title: 'Monthly' },
]

/**
 * Daily, Weekly and Monthly, in that order, each holding its habits in the order of the list
 * the backend sent (the order of `habits.toml`). The text narrows the rows by habit name.
 * A group without habits stays in the result with no items; the list does not draw it.
 */
export function groupHabits(list: readonly HabitSummary[], query: string): SelGroup<HabitRow>[] {
  return GROUPS.map(({ id, title }) => {
    const items = list
      .filter((summary) => summary.habit.cadence === id && matches(query, summary.habit.name))
      .map((summary) => ({ ...summary, id: summary.habit.id }))
    return { id, title, count: items.length, items }
  })
}
