// Number formatting: sizes, play time, token counts, file counts.

const BYTE_UNITS = ['B', 'KB', 'MB', 'GB', 'TB']
const grouped = new Intl.NumberFormat('en-US')

/** One decimal below 10, none from 10 up, and no trailing ".0": 4.1, 4, 38. */
function compact(n: number): string {
  return n < 10 ? String(Math.round(n * 10) / 10) : String(Math.round(n))
}

/** "38 GB", "412 MB", "4.1 GB". Uses 1024 steps, as Windows Explorer does. */
export function formatBytes(n: number): string {
  let value = Math.max(0, n)
  let unit = 0
  while (value >= 1024 && unit < BYTE_UNITS.length - 1) {
    value /= 1024
    unit += 1
  }
  // Rounding can reach 1024 ("1024 MB"); show that as the next unit.
  if (Math.round(value) >= 1024 && unit < BYTE_UNITS.length - 1) {
    value /= 1024
    unit += 1
  }
  return `${compact(value)} ${BYTE_UNITS[unit]}`
}

/** "35 min" below one hour, then hours: "1.5 h", "42 h". */
export function formatMinutes(n: number): string {
  const minutes = Math.max(0, Math.round(n))
  if (minutes < 60) return `${minutes} min`
  return `${compact(minutes / 60)} h`
}

/** Plain below 1000, then "1.5k", "48k", "1.2M". */
export function formatCount(n: number): string {
  if (n < 1000) return String(Math.round(n))
  if (n < 999_500) return `${compact(n / 1000)}k`
  return `${compact(n / 1_000_000)}M`
}

/** Thousands separators: "184,203". */
export function formatNumber(n: number): string {
  return grouped.format(n)
}
