// Game mode starts switched off with its five wind-down items. The counts and
// the "interrupts" marks are worked out from the live data when game mode
// starts (see slices/gameMode.ts), so the seed is the same in every scenario.

import type { GameModeState, WindDownItem } from '../../types'
import { lucide } from '../engine'
import type { Scenario } from '../options'

type ItemRow = Pick<WindDownItem, 'id' | 'label' | 'icon' | 'kind'>

/** In the order they are stopped. */
const ITEMS: ItemRow[] = [
  { id: 'wind-sessions', label: 'Claude sessions', icon: lucide('bot'), kind: 'session' },
  { id: 'wind-jobs', label: 'Scheduled jobs', icon: lucide('calendar-clock'), kind: 'job' },
  { id: 'wind-idle-jobs', label: 'Idle jobs', icon: lucide('cpu'), kind: 'idleJob' },
  { id: 'wind-service', label: 'Background service', icon: lucide('server'), kind: 'service' },
  { id: 'wind-window', label: 'Rain window', icon: lucide('app-window'), kind: 'window' },
]

export const DEFAULT_COUNTDOWN_SECONDS = 10

export function seed(_scenario: Scenario): GameModeState {
  return {
    phase: 'off',
    waiting: false,
    secondsLeft: DEFAULT_COUNTDOWN_SECONDS,
    countdownSeconds: DEFAULT_COUNTDOWN_SECONDS,
    items: ITEMS.map((row) => ({ ...row, count: 0, effect: 'safe', status: 'pending' })),
  }
}
