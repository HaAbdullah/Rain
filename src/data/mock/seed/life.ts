// Sample life data: habits read from daily notes in the vault, 400 trailing
// habit days and a few calendar events. Typical has 5 habits; "Meditate" is the
// one whose property is missing from the notes.

import { toISODate } from '@/lib/time'
import type { CalendarEvent, Habit, HabitDay, HabitValue, IconRef } from '../../types'
import { DAY, MINUTE, createRng, lucide } from '../engine'
import type { Scenario } from '../options'

export const VAULT_PATH = 'D:\\Vault'

// ---- habits ----

interface HabitRow {
  name: string
  icon: IconRef
  property: string
  target?: number
  unit?: string
  /** Value in today's note so far; leave out for "not filled in yet". */
  today?: HabitValue
}

const HABITS: HabitRow[] = [
  { name: 'Exercise', icon: lucide('dumbbell'), property: 'exercise', today: true },
  { name: 'Reading', icon: lucide('book-open'), property: 'reading', target: 30, unit: 'min', today: 12 },
  { name: 'Meditate', icon: lucide('brain'), property: 'meditate' },
  { name: 'Water', icon: lucide('glass-water'), property: 'water', target: 8, unit: 'glasses', today: 5 },
  { name: 'Journal', icon: lucide('notebook-pen'), property: 'journal' },
]

const BUSY_HABITS: HabitRow[] = [
  { name: 'Stretch', icon: lucide('person-standing'), property: 'stretch', today: true },
  { name: 'Steps', icon: lucide('footprints'), property: 'steps', target: 8000, unit: 'steps', today: 3400 },
]

/** Properties that no daily note contains. */
const MISSING_PROPERTIES = ['meditate']

const TRAILING_DAYS = 400
const NO_NOTE_RATE = 0.06
const SKIP_RATE = 0.03
/** The newest days always have a note, so current streaks are visible. */
const ALWAYS_NOTED_DAYS = 3

// ---- calendar ----

const PERSONAL = { calendar: 'Personal', color: 'var(--chart-2)' }
const WORK = { calendar: 'Work', color: 'var(--chart-1)' }

interface EventRow {
  title: string
  calendar: { calendar: string; color: string }
  /** Days from today. */
  day: number
  /** Start as a local clock time [hour, minute], or minutes from now for events placed around "now". */
  at: [number, number] | { fromNowMinutes: number }
  minutes: number
  location?: string
  allDay?: boolean
}

const EVENTS: EventRow[] = [
  { title: 'Recycling pickup', calendar: PERSONAL, day: 0, at: [0, 0], minutes: 24 * 60, allDay: true },
  { title: 'Stand-up', calendar: WORK, day: 0, at: { fromNowMinutes: -180 }, minutes: 15 },
  { title: 'Design review', calendar: WORK, day: 0, at: { fromNowMinutes: -20 }, minutes: 60, location: 'Room 2' },
  { title: 'Lunch walk', calendar: PERSONAL, day: 0, at: { fromNowMinutes: 90 }, minutes: 45, location: 'River path' },
  { title: 'Sprint planning', calendar: WORK, day: 0, at: { fromNowMinutes: 210 }, minutes: 60 },
  { title: 'Dentist', calendar: PERSONAL, day: 1, at: [9, 30], minutes: 45, location: 'Main Street Clinic' },
  { title: 'Release check-in', calendar: WORK, day: 1, at: [13, 0], minutes: 30 },
  { title: 'Board game night', calendar: PERSONAL, day: 1, at: [19, 0], minutes: 180 },
  { title: 'Quarterly review', calendar: WORK, day: 2, at: [11, 0], minutes: 90, location: 'Room 1' },
]

const BUSY_EVENTS: EventRow[] = [
  { title: 'Inbox sweep', calendar: WORK, day: 0, at: { fromNowMinutes: 45 }, minutes: 20 },
  { title: 'Pair on importer', calendar: WORK, day: 0, at: { fromNowMinutes: 300 }, minutes: 60 },
  { title: 'Groceries', calendar: PERSONAL, day: 0, at: { fromNowMinutes: 360 }, minutes: 40, location: 'Corner market' },
  { title: 'Morning run', calendar: PERSONAL, day: 1, at: [7, 0], minutes: 40 },
  { title: 'Roadmap sync', calendar: WORK, day: 1, at: [10, 30], minutes: 45 },
  { title: 'One-to-one', calendar: WORK, day: 1, at: [15, 0], minutes: 30 },
  { title: 'Library returns', calendar: PERSONAL, day: 1, at: [17, 30], minutes: 20 },
  { title: 'Team retro', calendar: WORK, day: 2, at: [9, 0], minutes: 60 },
  { title: 'Bike service', calendar: PERSONAL, day: 2, at: [12, 30], minutes: 30, location: 'Cycle shop' },
  { title: 'Demo rehearsal', calendar: WORK, day: 2, at: [14, 0], minutes: 45 },
  { title: 'Dinner with neighbours', calendar: PERSONAL, day: 2, at: [19, 30], minutes: 120 },
]

// ---- seed ----

export interface LifeSeed {
  /** Where the vault "exists" on disk; undefined when there is none to find. */
  vaultPath: string | undefined
  missingProperties: string[]
  habits: Habit[]
  /** Oldest first; the last entry is today. */
  habitDays: HabitDay[]
  events: CalendarEvent[]
}

export function seed(scenario: Scenario): LifeSeed {
  // Empty has no vault and no calendar. Failing has a calendar, but the vault cannot be found.
  const noVault = { vaultPath: undefined, missingProperties: [], habits: [], habitDays: [] }
  if (scenario === 'empty') return { ...noVault, events: [] }
  if (scenario === 'failing') return { ...noVault, events: EVENTS.map(toEvent) }

  const busy = scenario === 'busy'
  const rows = busy ? [...HABITS, ...BUSY_HABITS] : HABITS
  return {
    vaultPath: VAULT_PATH,
    missingProperties: MISSING_PROPERTIES,
    habits: rows.map(toHabit),
    habitDays: habitDays(rows),
    events: (busy ? [...EVENTS, ...BUSY_EVENTS] : EVENTS).map(toEvent),
  }
}

function habitId(row: HabitRow): string {
  return `habit-${row.property}`
}

function toHabit(row: HabitRow): Habit {
  return {
    id: habitId(row),
    name: row.name,
    icon: row.icon,
    property: row.property,
    kind: row.target === undefined ? 'boolean' : 'number',
    target: row.target,
    unit: row.unit,
  }
}

function habitDays(rows: HabitRow[]): HabitDay[] {
  const today = new Date()
  const days: HabitDay[] = []
  for (let back = TRAILING_DAYS - 1; back >= 0; back -= 1) {
    const date = new Date(today.getFullYear(), today.getMonth(), today.getDate() - back)
    days.push(back === 0 ? todayNote(date, rows) : pastDay(date, rows, back))
  }
  return days
}

/** Today's note exists and is filled in only as far as the day has got. */
function todayNote(date: Date, rows: HabitRow[]): HabitDay {
  const values: Record<string, HabitValue> = {}
  for (const row of rows) values[habitId(row)] = row.today ?? null
  return { date: toISODate(date), noteExists: true, values }
}

// Each day is generated from its own date, so a given date looks the same on every load.
function pastDay(date: Date, rows: HabitRow[], daysBack: number): HabitDay {
  const iso = toISODate(date)
  const noteExists = daysBack <= ALWAYS_NOTED_DAYS || !createRng(`note:${iso}`).chance(NO_NOTE_RATE)
  if (!noteExists) return { date: iso, noteExists, values: {} }

  const values: Record<string, HabitValue> = {}
  for (const row of rows) values[habitId(row)] = pastValue(row, date, iso)
  return { date: iso, noteExists, values }
}

function pastValue(row: HabitRow, date: Date, iso: string): HabitValue {
  if (MISSING_PROPERTIES.includes(row.property)) return null

  const rng = createRng(`${row.property}:${iso}`)
  if (rng.chance(SKIP_RATE)) return 'skip'

  const done = rng.chance(weekForm(row, date))
  if (row.target === undefined) {
    // A miss is usually an unticked box; now and then the property was left out.
    return done ? true : rng.chance(0.85) ? false : null
  }
  const share = done ? 1 + rng.next() * 0.5 : rng.next() * 0.9
  return Math.round(row.target * share)
}

/**
 * How likely the habit is done on a day of this week. Whole weeks share one
 * value, which gives long streaks with the odd bad patch instead of noise.
 */
function weekForm(row: HabitRow, date: Date): number {
  const week = Math.floor(Date.UTC(date.getFullYear(), date.getMonth(), date.getDate()) / (7 * DAY))
  const form = createRng(`${row.property}:week:${week}`).next()
  if (form < 0.12) return 0.3
  if (form < 0.4) return 0.7
  return 0.94
}

function toEvent(row: EventRow, index: number): CalendarEvent {
  const now = new Date()
  let start: Date
  if (Array.isArray(row.at)) {
    start = new Date(now.getFullYear(), now.getMonth(), now.getDate() + row.day, row.at[0], row.at[1])
  } else {
    // Rounded to five minutes so agenda times look like real appointments.
    start = new Date(Math.round((now.getTime() + row.at.fromNowMinutes * MINUTE) / (5 * MINUTE)) * 5 * MINUTE)
  }
  const end = new Date(start.getTime() + row.minutes * MINUTE)
  return {
    id: `event-${index + 1}`,
    title: row.title,
    start: start.toISOString(),
    end: end.toISOString(),
    allDay: row.allDay ?? false,
    calendar: row.calendar.calendar,
    color: row.calendar.color,
    location: row.location,
  }
}
