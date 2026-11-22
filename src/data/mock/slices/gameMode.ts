// Mock slice: the game-mode machine. Phases: off -> review (only when something
// would be interrupted) -> countdown -> windingDown -> done. The screen holds no
// timer of its own; every change arrives on the 'gameMode' topic.

import type { RainApi } from '../../api'
import type { GameModeState, WindDownItem } from '../../types'
import { SECOND, clone, delay, emit, every } from '../engine'
import { state } from '../store'
import { finishRunningJobs } from './jobs'
import { markGamePlayed } from './launches'
import { finishWorkingSessions } from './sessions'

const TICK_MS = 1 * SECOND
const WIND_DOWN_ITEM_MS = 500
/** "Wait for them": the interrupted work finishes after this many seconds in the mock. */
const WAIT_SECONDS = 5

/** Stops whichever ticker the current phase runs (waiting, countdown or wind-down). */
let stopTicker = (): void => {}

export const gameModeSlice: Pick<RainApi, 'getGameMode' | 'startGameMode' | 'resolveGameMode'> = {
  async getGameMode() {
    await delay()
    return clone(state.gameMode)
  },

  async startGameMode(opts) {
    await delay()
    const mode = state.gameMode
    // Already under way: a second start changes nothing.
    if (mode.phase === 'off' || mode.phase === 'done') {
      const seconds = state.settings.gameModeCountdownSeconds
      mode.waiting = false
      mode.countdownSeconds = seconds
      mode.secondsLeft = seconds
      mode.thenLaunchGameId = opts?.thenLaunchGameId
      mode.items = mode.items.map(measure)

      // The gate: the countdown does not start by itself while something would be interrupted.
      if (mode.items.some((item) => item.effect === 'interrupts')) mode.phase = 'review'
      else beginCountdown(mode)
      publish()
    }
    return clone(mode)
  },

  async resolveGameMode(choice) {
    await delay()
    const mode = state.gameMode
    const before = mode.phase

    if (choice === 'wait' && mode.phase === 'review' && !mode.waiting) beginWaiting(mode)
    if (choice === 'stopAnyway' && mode.phase === 'review') beginCountdown(mode)
    if (choice === 'closeNow' && mode.phase === 'countdown') beginWindDown(mode)
    // Wind-down cannot be cancelled: things are already being stopped.
    if (choice === 'cancel' && mode.phase !== 'windingDown' && mode.phase !== 'off') {
      stopTicker()
      mode.phase = 'off'
      mode.waiting = false
    }

    if (mode.phase !== before || mode.waiting) publish()
    return clone(mode)
  },
}

/** Fills in the live count of an item and whether stopping it loses work. */
function measure(item: WindDownItem): WindDownItem {
  let count = 1
  let interrupts = false
  if (item.kind === 'session') {
    count = state.sessions.filter((session) => session.status === 'running' || session.status === 'waiting').length
    interrupts = state.sessions.some((session) => session.status === 'running')
  } else if (item.kind === 'job') {
    count = state.jobs.filter((job) => job.enabled).length
    interrupts = state.jobs.some((job) => job.running)
  } else if (item.kind === 'idleJob') {
    count = state.idleJobs.filter((job) => job.enabled).length
    interrupts = state.idleJobs.some((job) => job.state === 'running')
  }
  return { ...item, count, effect: interrupts ? 'interrupts' : 'safe', status: 'pending' }
}

function beginWaiting(mode: GameModeState): void {
  mode.waiting = true
  let secondsWaited = 0
  stopTicker = every(TICK_MS, () => {
    secondsWaited += 1
    if (secondsWaited < WAIT_SECONDS) return

    // The work is done, so nothing is interrupted any more and the countdown can start.
    finishWorkingSessions()
    finishRunningJobs()
    for (const item of mode.items) item.effect = 'safe'
    beginCountdown(mode)
    publish()
  })
}

function beginCountdown(mode: GameModeState): void {
  stopTicker()
  mode.phase = 'countdown'
  mode.waiting = false
  mode.secondsLeft = mode.countdownSeconds
  stopTicker = every(TICK_MS, () => {
    mode.secondsLeft -= 1
    if (mode.secondsLeft <= 0) beginWindDown(mode)
    publish()
  })
}

/** Stops the items one after the other, 500 ms each, then game mode is done. */
function beginWindDown(mode: GameModeState): void {
  stopTicker()
  mode.phase = 'windingDown'
  mode.secondsLeft = 0
  let index = 0
  mode.items[index].status = 'stopping'
  stopTicker = every(WIND_DOWN_ITEM_MS, () => {
    mode.items[index].status = 'stopped'
    index += 1
    if (index < mode.items.length) {
      mode.items[index].status = 'stopping'
    } else {
      stopTicker()
      mode.phase = 'done'
      if (mode.thenLaunchGameId !== undefined) markGamePlayed(mode.thenLaunchGameId)
    }
    publish()
  })
}

function publish(): void {
  emit('gameMode', clone(state.gameMode))
}
