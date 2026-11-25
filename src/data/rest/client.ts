// The RainApi that talks to the Rust backend (foundation 8.3). One line per method on top of
// `call`; the paths are the route registry of foundation 14.4.
import type { Bridge, HttpMethod } from '@/shell/bridge'
import type { RainApi } from '../api'
import type {
  AppEntry, CalendarEvent, ClaudeSession, DiskNode, DiskScan, Drive, FocusResult, Game, GameLibrary, GameModeState, HardwareTile,
  Habit, HabitDay, HabitStats, HabitSummary, Health, IdleJob, JobRun, LaunchRun, LogLine, MachineState, OpenNoteResult,
  PathCheck, PathsCheckBody, PermissionAnswerResult, Pool, Project, PropertySuggestion, RevealBody,
  ScheduledJob, SearchItem, Settings, StartScanBody, VaultStatus,
} from '../types'
import { toRainError } from './errors'
import { resolveMedia } from './media'
import type { RestStream } from './stream'

const enc = encodeURIComponent

type QueryValue = string | number | string[] | undefined

/** `?a=1&b=2` from the defined values; an array becomes a comma list; nothing at all gives ''. */
function qs(params: Record<string, QueryValue>): string {
  const parts = Object.entries(params)
    .filter((entry): entry is [string, string | number | string[]] => entry[1] !== undefined)
    .map(([key, value]) => `${enc(key)}=${enc(Array.isArray(value) ? value.join(',') : String(value))}`)
  return parts.length === 0 ? '' : `?${parts.join('&')}`
}

/** `?a=1&b=2` from the values that are set; an undefined value is left out. Empty when none is set. */
function query(values: Record<string, string | undefined>): string {
  const parts = Object.entries(values)
    .filter((entry): entry is [string, string] => entry[1] !== undefined)
    .map(([key, value]) => `${encodeURIComponent(key)}=${encodeURIComponent(value)}`)
  return parts.length === 0 ? '' : `?${parts.join('&')}`
}

// `deliver` and `note` are optional so that a test with a bare stream still works; the real
// stream always has them.
export function createRestApi(
  bridge: Bridge,
  stream: Pick<RestStream, 'subscribe'> & Partial<Pick<RestStream, 'deliver' | 'note'>>,
): RainApi {
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
    // ---- settings ----
    getSettings: () => call<Settings>('GET', '/settings'),
    updateSettings: (patch) => call<Settings>('PATCH', '/settings', patch),
    // ---- health and streaming ----
    getHealth: () => call<Health>('GET', '/health'),
    // ---- projects ----
    listProjects: () => call<Project[]>('GET', '/projects'),
    subscribe: stream.subscribe,
    // ---- disk and machine ----
    listDrives: () => call<Drive[]>('GET', '/drives'),
    // The scan answers hold no media and can be large: `media: false` skips the image rewrite.
    startScan: (driveId) =>
      call<DiskScan>('POST', '/scans', { driveId } satisfies StartScanBody, { media: false }),
    getScan: (id) => call<DiskScan>('GET', `/scans/${encodeURIComponent(id)}`, undefined, { media: false }),
    getScanNode: (id, path) =>
      call<DiskNode>(
        'GET',
        `/scans/${encodeURIComponent(id)}/node?path=${encodeURIComponent(path)}`,
        undefined,
        { media: false },
      ),
    cancelScan: (id) => call<void>('DELETE', `/scans/${encodeURIComponent(id)}`),
    revealPath: (path) => call<void>('POST', '/shell/reveal', { path } satisfies RevealBody),
    checkPaths: (paths) => call<PathCheck[]>('POST', '/paths/check', { paths } satisfies PathsCheckBody),
    getMachineState: () => call<MachineState>('GET', '/machine'),
    // ---- hardware ----
    listHardware: () => call<HardwareTile[]>('GET', '/hardware'),
    openHardware: (id) => call<void>('POST', `/hardware/${enc(id)}/open`),

    // ---- launcher and apps, section 01 ----
    search: (query, types) => call<SearchItem[]>('GET', `/search${qs({ q: query, types })}`),
    listPinned: () => call<SearchItem[]>('GET', '/pinned'),
    listRecent: (limit) => call<SearchItem[]>('GET', `/recent${qs({ limit })}`),
    setPinned: (type, id, pinned) => call<void>('PUT', `/pinned/${type}/${enc(id)}`, { pinned }),
    listApps: () => call<AppEntry[]>('GET', '/apps'),
    rescanApps: () => call<AppEntry[]>('POST', '/apps/scan'),
    listPools: () => call<Pool[]>('GET', '/pools'),
    savePool: (draft) =>
      draft.id === undefined
        ? call<Pool>('POST', '/pools', draft)
        : call<Pool>('PUT', `/pools/${enc(draft.id)}`, draft),
    deletePool: (id) => call<void>('DELETE', `/pools/${enc(id)}`),
    launch: (target) => call<LaunchRun>('POST', '/launches', target),
    getLaunch: (id) => call<LaunchRun>('GET', `/launches/${enc(id)}`),

    // ---- real (projects and games, section 02) ----
    listGames: () => call<GameLibrary>('GET', '/games'),
    saveCustomGame: (draft) => call<Game>('POST', '/games', draft),
    // The answer body of a hide is not needed: the screens refetch.
    setGameHidden: (id, hidden) => call<Game>('PATCH', `/games/${encodeURIComponent(id)}`, { hidden }).then(() => undefined),
    updateGame: (id, patch) => call<Game>('PATCH', `/games/${encodeURIComponent(id)}`, patch),
    deleteCustomGame: (id) => call<void>('DELETE', `/games/${encodeURIComponent(id)}`),

    // ---- jobs and idle jobs, section 03 ----
    listJobs: () => call<ScheduledJob[]>('GET', '/jobs'),
    setJobEnabled: (id, enabled) =>
      call<ScheduledJob>('PATCH', `/jobs/${encodeURIComponent(id)}`, { enabled }),
    runJob: (id) => call<JobRun>('POST', `/jobs/${encodeURIComponent(id)}/runs`),
    getRunLog: (runId) =>
      call<LogLine[]>('GET', `/runs/${encodeURIComponent(runId)}/log`, undefined, { media: false }),
    listIdleJobs: () => call<IdleJob[]>('GET', '/idle-jobs'),
    setIdleJobEnabled: (id, enabled) =>
      call<IdleJob>('PATCH', `/idle-jobs/${encodeURIComponent(id)}`, { enabled }),
    runIdleJob: (id) => call<IdleJob>('POST', `/idle-jobs/${encodeURIComponent(id)}/run`),
    getIdleJobLog: (id) =>
      call<LogLine[]>('GET', `/idle-jobs/${encodeURIComponent(id)}/log`, undefined, { media: false }),

    // ---- Claude sessions ----
    listSessions: () => call<ClaudeSession[]>('GET', '/sessions'),
    answerPermission: (sessionId, requestId, answer) =>
      call<PermissionAnswerResult>('POST', `/sessions/${encodeURIComponent(sessionId)}/permission`, { requestId, answer }),
    focusSession: (id) => call<FocusResult>('POST', `/sessions/${encodeURIComponent(id)}/focus`),
    stopSession: (id) => call<void>('POST', `/sessions/${encodeURIComponent(id)}/stop`),
    getSessionTranscript: (id) =>
      call<LogLine[]>('GET', `/sessions/${encodeURIComponent(id)}/transcript`, undefined, { media: false }),

    // ---- real (life, section 04) ----
    getVaultStatus: () => call<VaultStatus>('GET', '/vault'),
    rereadVault: () => call<void>('POST', '/vault/reread'),
    listHabitSuggestions: () => call<PropertySuggestion[]>('GET', '/vault/properties'),
    listHabits: () => call<HabitSummary[]>('GET', '/habits'),
    saveHabit: (draft) =>
      draft.id
        ? call<Habit>('PUT', `/habits/${encodeURIComponent(draft.id)}`, draft)
        : call<Habit>('POST', '/habits', draft),
    deleteHabit: (id) => call<void>('DELETE', `/habits/${encodeURIComponent(id)}`),
    getHabitStats: (id) => call<HabitStats>('GET', `/habits/${encodeURIComponent(id)}/stats`),
    listHabitDays: (from, to, habitId) =>
      call<HabitDay[]>('GET', `/habit-days${query({ from, to, habit: habitId })}`),
    setHabitDone: (id, date, done) =>
      call<HabitSummary>('PUT', `/habits/${encodeURIComponent(id)}/days/${encodeURIComponent(date)}`, { done }),
    listEvents: (from, to) => call<CalendarEvent[]>('GET', `/calendar/events${query({ from, to })}`),
    openDailyNote: (date) => call<OpenNoteResult>('POST', `/daily-notes/${encodeURIComponent(date)}/open`),

    // ---- game mode ----
    // The state in the answer reaches the subscribers before the call resolves, and a state
    // that is older than one already applied is dropped (section 06 6.10).
    getGameMode: async () => {
      const state = await call<GameModeState>('GET', '/game-mode')
      return stream.note?.('gameMode', state) ?? state
    },
    startGameMode: async (opts) => {
      const state = await call<GameModeState>('POST', '/game-mode', opts ?? {})
      return stream.deliver?.('gameMode', state) ?? state
    },
    resolveGameMode: async (choice) => {
      const state = await call<GameModeState>('POST', '/game-mode/choice', { choice })
      return stream.deliver?.('gameMode', state) ?? state
    },
    markGameModeRecapSeen: () => call<void>('POST', '/game-mode/recap/seen'),
  }
}
