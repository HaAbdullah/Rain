// Actions for a habit row and for a day of the open habit (spec 4.3), the hooks of the habit
// dialog and of the full view, and "open today's note", which Ctrl+T and a command also use.
//
// Rain reads the vault and writes one thing: the tick below asks the backend to set the one
// property of one daily note. Nothing here writes a note itself.
import { ref } from 'vue'
import { hideAfterSuccess } from '@/actions/common'
import type { ActionContext, ActionProvider, RainAction, TargetOf } from '@/actions/types'
import { refreshResources } from '@/composables/useResource'
import { useViewState } from '@/composables/useViewState'
import type { Habit, HabitDraft, ID, ISODate } from '@/data/types'
import { toISODate } from '@/lib/time'

/** The open habit of the Life screen. '' is the list. Read by LifeScreen and set by the actions. */
export const lifeView = useViewState('life.open', { habitId: '' })

/** The day the cursor is on in the full view. '' means today, so it follows the calendar past midnight. */
export const lifeCursor = useViewState('life.cursor', { date: '' })

/** Opens the full view of a habit, with the cursor on today. */
export function openHabit(id: ID): void {
  lifeCursor.date = ''
  lifeView.habitId = id
}

export function closeHabit(): void {
  lifeView.habitId = ''
}

/**
 * The state of the habit dialog. HabitEditorDialog (mounted by the Life screen) is open while
 * `draft` is not null; it saves with api.saveHabit and sets it back to null.
 */
export const habitEditor = {
  draft: ref<HabitDraft | null>(null),

  /** Ctrl+N and the "New habit" command. */
  create() {
    habitEditor.draft.value = {
      name: '',
      icon: { kind: 'lucide', value: 'dumbbell' },
      property: '',
      cadence: 'daily',
      times: 1,
    }
  },

  edit(habit: Habit) {
    // A copy, so Cancel leaves the habit as it was.
    habitEditor.draft.value = { ...habit, icon: { ...habit.icon } }
  },
}

async function openNote(date: ISODate, missing: string, ctx: ActionContext): Promise<void> {
  const { opened } = await ctx.api.openDailyNote(date)
  if (opened) await hideAfterSuccess(ctx)
  else ctx.notice.info(missing)
}

/** Opens today's daily note, then hides. Used by Ctrl+T, the command and the habit actions. Never creates a note. */
export function openTodaysNote(ctx: ActionContext): Promise<void> {
  return openNote(toISODate(new Date()), 'No note for today', ctx)
}

/** One tick at a time per habit and day: a held Enter must not send a second request while one is on its way. */
const pending = new Set<string>()

async function setDone(ctx: ActionContext, habit: Habit, date: ISODate, done: boolean) {
  const key = `${habit.id}:${date}`
  if (pending.has(key)) return
  pending.add(key)
  try {
    await ctx.api.setHabitDone(habit.id, date, done)
    refreshResources('habits', 'habitDays', 'habitStats')
  } finally {
    pending.delete(key)
  }
}

/** The dialog is mounted by the Life screen, so go there first. */
function editHabit(habit: Habit, ctx: ActionContext) {
  if (ctx.nav.displayed.value !== 'life') ctx.nav.go('life')
  habitEditor.edit(habit)
}

/** Ctrl+R: look at the notes again (the backend re-reads and the screens refetch). */
const reread: RainAction = {
  id: 'reread',
  label: 'Re-read vault',
  icon: 'refresh-cw',
  slot: 'more',
  keys: 'Ctrl+KeyR',
  async run({ api, notice }) {
    await api.rereadVault()
    refreshResources('vault', 'habits', 'habitDays', 'habitStats')
    notice.info('Reading the vault again')
  },
}

export const habitActions: ActionProvider<TargetOf<'habit'>> = ({ item: summary }) => [
  {
    id: 'tick',
    label: summary.todayDone ? 'Untick today' : 'Tick today',
    icon: summary.todayDone ? 'square' : 'square-check',
    slot: 'primary',
    run: (ctx) => setDone(ctx, summary.habit, summary.today, !summary.todayDone),
  },
  {
    id: 'open-habit',
    label: 'Open habit',
    icon: 'chart-column',
    slot: 'secondary',
    run() {
      openHabit(summary.habit.id)
    },
  },
  {
    id: 'edit-habit',
    label: 'Edit habit',
    icon: 'pencil',
    slot: 'more',
    run: (ctx) => editHabit(summary.habit, ctx),
  },
  { id: 'open-today', label: "Open today's note", icon: 'notebook-pen', slot: 'more', run: openTodaysNote },
  reread,
  {
    id: 'delete-habit',
    label: 'Delete habit',
    icon: 'trash-2',
    slot: 'more',
    danger: true,
    async run({ api, confirm, notice }) {
      const confirmed = await confirm({
        title: `Delete ${summary.habit.name}?`,
        body: `The habit is removed from Rain. Your notes keep their '${summary.habit.property}' property.`,
        confirmLabel: 'Delete habit',
        danger: true,
      })
      if (!confirmed) return
      await api.deleteHabit(summary.habit.id)
      if (lifeView.habitId === summary.habit.id) closeHabit()
      refreshResources('habits', 'habitDays', 'habitStats')
      notice.success(`Deleted ${summary.habit.name}`)
    },
  },
]

export const habitDayActions: ActionProvider<TargetOf<'habitDay'>> = ({ item: day }) => [
  {
    id: 'tick',
    label: day.done ? 'Untick this day' : 'Tick this day',
    icon: day.done ? 'square' : 'square-check',
    slot: 'primary',
    async run(ctx) {
      if (!day.tickable) ctx.notice.info('That day is too old for Rain to change. Open its note instead.')
      else await setDone(ctx, day.habit, day.date, !day.done)
    },
  },
  {
    id: 'open-note',
    label: "Open that day's note",
    icon: 'notebook-pen',
    slot: 'secondary',
    async run(ctx) {
      // Known from the data already: no need to ask the api for a note that is not there.
      if (!day.noteExists) ctx.notice.info('No note for this day')
      else await openNote(day.date, 'No note for this day', ctx)
    },
  },
  { id: 'open-today', label: "Open today's note", icon: 'calendar-check', slot: 'more', run: openTodaysNote },
  {
    id: 'edit-habit',
    label: 'Edit habit',
    icon: 'pencil',
    slot: 'more',
    run: (ctx) => editHabit(day.habit, ctx),
  },
  reread,
]
