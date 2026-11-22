// Mock slice: the read-only Life data (vault status, habits, habit days,
// calendar events). Nothing here ever writes: Rain only reads the vault.

import { toISODate } from '@/lib/time'
import type { RainApi } from '../../api'
import { clone, delay, fail } from '../engine'
import { options } from '../options'
import { state } from '../store'

type LifeSlice = Pick<RainApi, 'getVaultStatus' | 'listHabits' | 'listHabitDays' | 'listEvents' | 'openDailyNote'>

export const lifeSlice: LifeSlice = {
  async getVaultStatus() {
    await delay()
    const connected = vaultConnected()
    return {
      connected,
      path: state.settings.vaultPath || undefined,
      missingProperties: connected ? [...state.missingProperties] : [],
    }
  },

  async listHabits() {
    await delay()
    return vaultConnected() ? clone(state.habits) : []
  },

  async listHabitDays(from, to) {
    await delay()
    if (!vaultConnected()) return []
    // ISO dates compare correctly as text.
    return clone(state.habitDays.filter((day) => day.date >= from && day.date <= to))
  },

  async listEvents(from, to) {
    await delay()
    if (options.scenario === 'failing') throw fail('unavailable', 'Calendar unavailable', 'calendar')
    if (!state.settings.calendarConnected) return []
    return clone(state.events.filter((event) => {
      const day = toISODate(new Date(event.start))
      return day >= from && day <= to
    }))
  },

  async openDailyNote(date) {
    await delay()
    if (!vaultConnected()) throw fail('unavailable', 'Obsidian vault not found', 'vault')
    const day = state.habitDays.find((candidate) => candidate.date === date)
    // Rain never creates a note: a day without one simply does not open.
    return { opened: day?.noteExists ?? false }
  },
}

/** The vault is found when the path in Settings points at the folder the mock vault "lives" in. */
function vaultConnected(): boolean {
  return state.vaultPath !== undefined && samePath(state.settings.vaultPath, state.vaultPath)
}

function samePath(a: string, b: string): boolean {
  const tidy = (path: string) => path.trim().replace(/[\\/]+$/, '').toLowerCase()
  return tidy(a) === tidy(b)
}
