// The one service interface the UI talks to. Every method returns a Promise and
// maps to one REST call. The routes are in docs/backend/00-foundation.md section 14.4;
// the comments below are the prototype's and are replaced stage by stage.
// Rejections are always a RainError.

import type {
  ApiError, AppEntry, CalendarEvent, ClaudeSession, CustomGameDraft, DiskScan, Drive, Game,
  GameLibrary, GameModeChoice, GameModeState, Habit, HabitDay, Health, ID, IdleJob, ISODate, JobRun,
  LaunchRun, LaunchTarget, LogLine, MachineState, PermissionAnswer, PinnableType, Pool, PoolDraft,
  Project, ScheduledJob, SearchItem, SearchType, Settings, SettingsPatch, VaultStatus,
} from './types'

/** What every rejected call carries: the backend's error body (or one the bridge made in its shape). */
export type RainError = ApiError

export interface TopicPayload {
  sessions: ClaudeSession[]
  jobs: ScheduledJob[]
  idleJobs: IdleJob[]
  launch: LaunchRun
  scan: DiskScan
  gameMode: GameModeState
  settings: Settings
  health: Health
}
export type Topic = keyof TopicPayload

export interface RainApi {
  // search and pins
  // Home merges these with command scores from lib/score.ts and sorts them together, so
  // SearchItem.score must use that scale (0 = no match, higher is better).
  search(query: string, types?: SearchType[]): Promise<SearchItem[]>          // GET  /search?q=&types=
  listPinned(): Promise<SearchItem[]>                                          // GET  /pinned
  listRecent(limit: number): Promise<SearchItem[]>                             // GET  /recent?limit=
  setPinned(type: PinnableType, id: ID, pinned: boolean): Promise<void>        // PUT  /pinned/:type/:id

  // apps and pools
  listApps(): Promise<AppEntry[]>                                              // GET  /apps
  listPools(): Promise<Pool[]>                                                 // GET  /pools
  savePool(draft: PoolDraft): Promise<Pool>                                    // POST /pools | PUT /pools/:id
  deletePool(id: ID): Promise<void>                                            // DELETE /pools/:id

  // launching (apps, pools, projects, games)
  launch(target: LaunchTarget): Promise<LaunchRun>                             // POST /launches
  getLaunch(id: ID): Promise<LaunchRun>                                        // GET  /launches/:id

  // projects and games
  listProjects(): Promise<Project[]>                                           // GET  /projects
  listGames(): Promise<GameLibrary>                                            // GET  /games
  saveCustomGame(draft: CustomGameDraft): Promise<Game>                        // POST /games
  setGameHidden(id: ID, hidden: boolean): Promise<void>                        // PATCH /games/:id

  // Claude sessions
  listSessions(): Promise<ClaudeSession[]>                                     // GET  /sessions
  answerPermission(sessionId: ID, answer: PermissionAnswer): Promise<ClaudeSession> // POST /sessions/:id/permission
  focusSession(id: ID): Promise<void>                                          // POST /sessions/:id/focus
  stopSession(id: ID): Promise<void>                                           // POST /sessions/:id/stop
  getSessionTranscript(id: ID): Promise<LogLine[]>                             // GET  /sessions/:id/transcript

  // scheduled jobs
  listJobs(): Promise<ScheduledJob[]>                                          // GET  /jobs
  setJobEnabled(id: ID, enabled: boolean): Promise<ScheduledJob>               // PATCH /jobs/:id
  runJob(id: ID): Promise<JobRun>                                              // POST /jobs/:id/runs
  getRunLog(runId: ID): Promise<LogLine[]>                                     // GET  /runs/:id/log

  // idle jobs
  listIdleJobs(): Promise<IdleJob[]>                                           // GET  /idle-jobs
  setIdleJobEnabled(id: ID, enabled: boolean): Promise<IdleJob>                // PATCH /idle-jobs/:id
  runIdleJob(id: ID): Promise<IdleJob>                                         // POST /idle-jobs/:id/run
  getIdleJobLog(id: ID): Promise<LogLine[]>                                    // GET  /idle-jobs/:id/log
  getMachineState(): Promise<MachineState>                                     // GET  /machine

  // life (read-only)
  getVaultStatus(): Promise<VaultStatus>                                       // GET  /vault
  listHabits(): Promise<Habit[]>                                               // GET  /habits
  listHabitDays(from: ISODate, to: ISODate): Promise<HabitDay[]>               // GET  /habit-days?from=&to=
  listEvents(from: ISODate, to: ISODate): Promise<CalendarEvent[]>             // GET  /calendar/events?from=&to=
  openDailyNote(date: ISODate): Promise<{ opened: boolean }>                   // POST /daily-notes/:date/open

  // disk
  listDrives(): Promise<Drive[]>                                               // GET  /drives
  startScan(driveId: ID): Promise<DiskScan>                                    // POST /scans
  getScan(id: ID): Promise<DiskScan>                                           // GET  /scans/:id
  cancelScan(id: ID): Promise<void>                                            // DELETE /scans/:id
  revealPath(path: string): Promise<void>                                      // POST /shell/reveal

  // game mode
  getGameMode(): Promise<GameModeState>                                        // GET  /game-mode
  startGameMode(opts?: { thenLaunchGameId?: ID }): Promise<GameModeState>      // POST /game-mode
  resolveGameMode(choice: GameModeChoice): Promise<GameModeState>              // POST /game-mode/choice

  // settings
  getSettings(): Promise<Settings>                                             // GET  /settings
  updateSettings(patch: SettingsPatch): Promise<Settings>                        // PATCH /settings
  getHealth(): Promise<Health>                                                  // GET  /health (config issues, source states)

  // live updates (one server-sent stream, GET /stream)
  subscribe<T extends Topic>(topic: T, cb: (payload: TopicPayload[T]) => void): () => void
}
