// The order of "All projects" (spec 7.3): the project opened most recently comes first.
import type { Project } from '@/data/types'

/** When the project was last opened from Rain; 0 for one that never was. */
export function openedAt(project: Project): number {
  return Date.parse(project.lastOpenedAt ?? '') || 0
}

/**
 * Comparator for `Array.sort`. Projects that were never opened all tie at 0 and come after the
 * opened ones. `sort` is stable, so they keep the order the backend gave them (activity, then name).
 */
export function byLastOpened(a: Project, b: Project): number {
  return openedAt(b) - openedAt(a)
}
