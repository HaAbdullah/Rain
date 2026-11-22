// Date and time formatting for the whole app. Plain Date and Intl only, so no
// date library is needed. Every function is pure: "now" is always passed in.

const SECOND = 1000
const MINUTE = 60 * SECOND
const HOUR = 60 * MINUTE
const DAY = 24 * HOUR

const WEEKDAYS = ['Sun', 'Mon', 'Tue', 'Wed', 'Thu', 'Fri', 'Sat']
const MONTHS = ['Jan', 'Feb', 'Mar', 'Apr', 'May', 'Jun', 'Jul', 'Aug', 'Sep', 'Oct', 'Nov', 'Dec']

function pad2(n: number): string {
  return String(n).padStart(2, '0')
}

/** Size of a time span as a number and a one-letter unit: 9 h, 5 d, 3 w. */
function spanParts(ms: number): { n: number; unit: string } {
  if (ms < HOUR) return { n: Math.floor(ms / MINUTE), unit: 'm' }
  if (ms < DAY) return { n: Math.floor(ms / HOUR), unit: 'h' }
  if (ms < 14 * DAY) return { n: Math.floor(ms / DAY), unit: 'd' }
  if (ms < 60 * DAY) return { n: Math.floor(ms / (7 * DAY)), unit: 'w' }
  if (ms < 365 * DAY) return { n: Math.floor(ms / (30 * DAY)), unit: 'mo' }
  return { n: Math.floor(ms / (365 * DAY)), unit: 'y' }
}

/**
 * Relative time against `now` (epoch ms).
 * Past: "now", "2m", "3h", "yesterday", "5d", "3w". Future: "in 12 m", "in 9 h", "tomorrow".
 * `short` keeps every value to a few characters: "1d" instead of "yesterday", "in 9h".
 */
export function relTime(iso: string, now: number, short = false): string {
  const then = Date.parse(iso)
  if (Number.isNaN(then)) return ''

  const diff = now - then
  const span = Math.abs(diff)
  if (span < MINUTE) return 'now'

  const future = diff < 0
  const { n, unit } = spanParts(span)

  if (!short && unit === 'd' && n === 1) return future ? 'tomorrow' : 'yesterday'
  if (!future) return `${n}${unit}`
  return short ? `in ${n}${unit}` : `in ${n} ${unit}`
}

/** The LOCAL calendar date as YYYY-MM-DD (toISOString would give the UTC date). */
export function toISODate(d: Date): string {
  return `${d.getFullYear()}-${pad2(d.getMonth() + 1)}-${pad2(d.getDate())}`
}

/** Local wall-clock time, 24 h: "06:00". */
export function formatClock(iso: string): string {
  const d = new Date(iso)
  if (Number.isNaN(d.getTime())) return ''
  return `${pad2(d.getHours())}:${pad2(d.getMinutes())}`
}

/** "Sat 21 Nov" for a YYYY-MM-DD date (a full date-time is accepted too). */
export function formatDayLabel(isoDate: string): string {
  // A bare date is built from its parts: new Date('2026-11-21') would be read as UTC
  // and can land on the day before in time zones west of Greenwich.
  const bare = /^(\d{4})-(\d{2})-(\d{2})$/.exec(isoDate)
  const d = bare ? new Date(Number(bare[1]), Number(bare[2]) - 1, Number(bare[3])) : new Date(isoDate)
  if (Number.isNaN(d.getTime())) return ''
  return `${WEEKDAYS[d.getDay()]} ${d.getDate()} ${MONTHS[d.getMonth()]}`
}

/** A run time: "42 s", "3 m 10 s", "1 h 5 m". */
export function formatDuration(ms: number): string {
  if (ms > 0 && ms < SECOND) return '<1 s'

  const total = Math.max(0, Math.round(ms / SECOND))
  const h = Math.floor(total / 3600)
  const m = Math.floor((total % 3600) / 60)
  const s = total % 60

  if (h > 0) return m > 0 ? `${h} h ${m} m` : `${h} h`
  if (m > 0) return s > 0 ? `${m} m ${s} s` : `${m} m`
  return `${s} s`
}
