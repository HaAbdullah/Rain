// The RainApi that talks to the Rust backend (foundation 8.3). One line per method on top of
// `call`; the paths are the route registry of foundation 14.4. A method whose stage has not
// been built yet is a stub, and the router (../router.ts) never sends a call to a stub.
import type { Bridge, HttpMethod } from '@/shell/bridge'
import type { RainApi, RainError } from '../api'
import type { Health, Settings } from '../types'
import { toRainError } from './errors'
import { resolveMedia } from './media'
import type { RestStream } from './stream'

const NOT_IMPLEMENTED: RainError = {
  code: 'unavailable',
  message: 'Not connected yet',
  source: 'client',
  reason: 'notImplemented',
}

const notImplemented = () => Promise.reject<never>(NOT_IMPLEMENTED)

export function createRestApi(bridge: Bridge, stream: Pick<RestStream, 'subscribe'>): RainApi {
  /**
   * One call to the backend. Never retries: the Retry button of a screen does that.
   * Images in the answer become loadable addresses unless the call says `media: false`
   * (large answers that have none, such as disk scans).
   */
  async function call<T>(
    method: HttpMethod,
    path: string,
    body?: unknown,
    options: { media?: boolean } = {},
  ): Promise<T> {
    let answer: unknown
    try {
      answer = await bridge.request({ method, path, body })
    } catch (reason) {
      throw toRainError(reason)
    }
    return (options.media === false ? answer : resolveMedia(answer, bridge.mediaUrl)) as T
  }

  return {
    // ---- real (stage 1b) ----
    getSettings: () => call<Settings>('GET', '/settings'),
    updateSettings: (patch) => call<Settings>('PATCH', '/settings', patch),
    // ---- real (stage 2) ----
    getHealth: () => call<Health>('GET', '/health'),
    subscribe: stream.subscribe,

    // ---- not built yet: each stage replaces the stubs of its routes ----
    search: notImplemented,
    listPinned: notImplemented,
    listRecent: notImplemented,
    setPinned: notImplemented,
    listApps: notImplemented,
    listPools: notImplemented,
    savePool: notImplemented,
    deletePool: notImplemented,
    launch: notImplemented,
    getLaunch: notImplemented,
    listProjects: notImplemented,
    listGames: notImplemented,
    saveCustomGame: notImplemented,
    setGameHidden: notImplemented,
    listSessions: notImplemented,
    answerPermission: notImplemented,
    focusSession: notImplemented,
    stopSession: notImplemented,
    getSessionTranscript: notImplemented,
    listJobs: notImplemented,
    setJobEnabled: notImplemented,
    runJob: notImplemented,
    getRunLog: notImplemented,
    listIdleJobs: notImplemented,
    setIdleJobEnabled: notImplemented,
    runIdleJob: notImplemented,
    getIdleJobLog: notImplemented,
    getMachineState: notImplemented,
    getVaultStatus: notImplemented,
    listHabits: notImplemented,
    listHabitDays: notImplemented,
    listEvents: notImplemented,
    openDailyNote: notImplemented,
    listDrives: notImplemented,
    startScan: notImplemented,
    getScan: notImplemented,
    cancelScan: notImplemented,
    revealPath: notImplemented,
    getGameMode: notImplemented,
    startGameMode: notImplemented,
    resolveGameMode: notImplemented,
  }
}
