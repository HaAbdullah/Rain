// The pure parts of the habit dialog: which icons it offers, how a property name is made from a
// habit name, and how many times a cadence allows. Kept out of the component so they can be tested.
import type { Cadence } from '@/data/types'

/** The icons the dialog offers. The first one is the default of a new habit. All are in lib/icon-map.ts. */
export const HABIT_ICONS = [
  'dumbbell', 'book-open', 'brain', 'glass-water', 'footprints', 'person-standing', 'notebook-pen',
  'moon', 'pill', 'sun', 'heart-pulse', 'bike', 'droplets', 'leaf', 'salad', 'coffee', 'bed',
  'guitar', 'graduation-cap', 'flame', 'smile', 'utensils', 'mountain', 'music', 'sunrise', 'timer',
  'target', 'waves',
] as const

/** `Gym time` becomes `gym-time`: lower case, spaces and underscores to dashes, nothing else odd. */
export function propertyFromName(name: string): string {
  return name
    .trim()
    .toLowerCase()
    .replace(/[\s_]+/g, '-')
    .replace(/[^\p{L}\p{N}-]/gu, '')
    .replace(/-+/g, '-')
    .replace(/^-|-$/g, '')
}

/** `morning-run` becomes `Morning run`: a name to start from when a chip fills the property. */
export function nameFromProperty(property: string): string {
  const words = property.replace(/[-_]+/g, ' ').trim()
  return words === '' ? '' : words[0].toUpperCase() + words.slice(1)
}

/** The most "times" a cadence allows: 7 a week, 31 a month, and a daily habit is done once. */
export function timesMax(cadence: Cadence): number {
  return cadence === 'weekly' ? 7 : cadence === 'monthly' ? 31 : 1
}

/** A typed number held to what the cadence allows (1 to the maximum). Not a number gives 1. */
export function clampTimes(cadence: Cadence, value: number): number {
  if (!Number.isFinite(value)) return 1
  return Math.min(timesMax(cadence), Math.max(1, Math.round(value)))
}
