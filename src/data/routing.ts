// TEMPORARY (deleted in stage 10): which side answers each call while the backend is built
// stage by stage. This is the only file a stage edits to move a method (foundation 8.6).
// The Record types make a method or topic that is missing here a compile error.
import type { RainApi, Topic } from './api'
import type { RetiredMockMethod } from './mock'

export type Side = 'real' | 'mock'

export type MethodName = keyof Omit<RainApi, 'subscribe'>

/** A method the mock has retired can only be answered by the backend: 'mock' is a compile error for it. */
type SideOf<M extends MethodName> = M extends RetiredMockMethod ? 'real' : Side

export interface Routing {
  methods: { [M in MethodName]: SideOf<M> }
  topics: Record<Topic, Side>
}

export const routing: Routing = {
  // Grouped by feature section, so that a section flips only its own lines.
  methods: {
    // launcher and apps (section 01)
    search: 'mock',
    listPinned: 'mock',
    listRecent: 'mock',
    setPinned: 'mock',
    listApps: 'mock',
    listPools: 'mock',
    savePool: 'mock',
    deletePool: 'mock',
    launch: 'mock',
    getLaunch: 'mock',
    // projects and games (section 02)
    listProjects: 'mock',
    listGames: 'mock',
    saveCustomGame: 'mock',
    setGameHidden: 'mock',
    // sessions, jobs and idle jobs (section 03)
    listSessions: 'mock',
    answerPermission: 'mock',
    focusSession: 'mock',
    stopSession: 'mock',
    getSessionTranscript: 'mock',
    listJobs: 'mock',
    setJobEnabled: 'mock',
    runJob: 'mock',
    getRunLog: 'mock',
    listIdleJobs: 'mock',
    setIdleJobEnabled: 'mock',
    runIdleJob: 'mock',
    getIdleJobLog: 'mock',
    // life (section 04)
    getVaultStatus: 'mock',
    listHabits: 'mock',
    listHabitDays: 'mock',
    listEvents: 'mock',
    openDailyNote: 'mock',
    // disk and machine (section 05)
    getMachineState: 'mock',
    listDrives: 'mock',
    startScan: 'mock',
    getScan: 'mock',
    cancelScan: 'mock',
    revealPath: 'mock',
    // game mode (section 06)
    getGameMode: 'mock',
    startGameMode: 'mock',
    resolveGameMode: 'mock',
    // settings and health (core, real since stage 1b and 2)
    getSettings: 'real',
    updateSettings: 'real',
    getHealth: 'real',
  },
  topics: {
    launch: 'mock', // section 01
    sessions: 'mock', // section 03
    jobs: 'mock', // section 03
    idleJobs: 'mock', // section 03
    scan: 'mock', // section 05
    gameMode: 'mock', // section 06
    settings: 'real',
    health: 'real',
  },
}
