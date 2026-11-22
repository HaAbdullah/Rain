// Actions for a day cell in Life (spec 4.3), and "open today's note", which
// Ctrl+T and a command also use. Rain only reads the vault: a missing note is
// reported, never created.
import { hideAfterSuccess } from '@/actions/common'
import type { ActionContext, ActionProvider, TargetOf } from '@/actions/types'
import { refreshResources } from '@/composables/useResource'
import type { ISODate } from '@/data/types'
import { toISODate } from '@/lib/time'

async function openNote(date: ISODate, missing: string, ctx: ActionContext): Promise<void> {
  const { opened } = await ctx.api.openDailyNote(date)
  if (opened) await hideAfterSuccess(ctx)
  else ctx.notice.info(missing)
}

/** Opens today's daily note, then hides. Used by Ctrl+T, the command and the day actions. */
export function openTodaysNote(ctx: ActionContext): Promise<void> {
  return openNote(toISODate(new Date()), 'No note for today', ctx)
}

export const dayActions: ActionProvider<TargetOf<'day'>> = ({ item: day }) => [
  {
    id: 'open-note',
    label: "Open that day's note",
    icon: 'notebook-pen',
    slot: 'primary',
    async run(ctx) {
      // Known from the data already: no need to ask the api for a note that is not there.
      if (!day.noteExists) ctx.notice.info('No note for this day')
      else await openNote(day.date, 'No note for this day', ctx)
    },
  },
  {
    id: 'open-today',
    label: "Open today's note",
    icon: 'calendar-check',
    slot: 'secondary',
    run: openTodaysNote,
  },
  {
    id: 'reread',
    label: 'Re-read vault',
    icon: 'refresh-cw',
    slot: 'more',
    keys: 'Ctrl+KeyR',
    run({ notice }) {
      refreshResources('vault', 'habits', 'habitDays', 'events')
      notice.info('Reading the vault again')
    },
  },
]
