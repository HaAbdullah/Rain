// The one service interface the UI talks to. Every method returns a Promise and
// maps to one REST call; the route is in the comment of each method and in
// docs/backend/00-foundation.md section 14.4. Rejections are always a RainError.

import type {
  ApiError, AppEntry, CalendarEvent, ClaudeSession, CustomGameDraft, DiskNode, DiskScan, Drive, FocusResult, Game,
  GameLibrary, GameModeChoice, HardwareTile, GameModeState, Habit, HabitDay, HabitDraft, HabitStats, HabitSummary, Health, ID, IdleJob, ISODate, JobRun,
  LaunchRun, LaunchTarget, LogLine, MachineState, OpenNoteResult, PathCheck, PermissionAnswer, PermissionAnswerResult, PinnableType, Pool, PoolDraft,
  GamePatch, Project, PropertySuggestion, ScheduledJob, SearchItem, SearchType, Settings, SettingsPatch, VaultStatus,
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
  projects: Project[]
  games: GameLibrary
  habits: HabitSummary[]
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
  rescanApps(): Promise<AppEntry[]>                                           // POST /apps/scan
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
  updateGame(id: ID, patch: GamePatch): Promise<Game>                          // PATCH /games/:id
  deleteCustomGame(id: ID): Promise<void>                                      // DELETE /games/:id

  // Claude sessions
  // Live sessions plus the ones that ended in the last 24 hours (newest first, at most 40).
  listSessions(): Promise<ClaudeSession[]>                                     // GET  /sessions
  // The UI sends the id of the request it shows. An expired request is a 409 conflict, reason
  // requestExpired. 'always' lasts for this session only and the result says what was applied.
  answerPermission(sessionId: ID, requestId: ID, answer: PermissionAnswer): Promise<PermissionAnswerResult> // POST /sessions/:id/permission
  // 'none' is not an error: the window could not be found or Windows refused to bring it forward.
  focusSession(id: ID): Promise<FocusResult>                                   // POST /sessions/:id/focus
  stopSession(id: ID): Promise<void>                                           // POST /sessions/:id/stop (live terminal sessions only)
  getSessionTranscript(id: ID): Promise<LogLine[]>                             // GET  /sessions/:id/transcript

  // scheduled jobs (section 03 2.2). The definitions are jobs.toml in the data folder; the app only
  // toggles `enabled` and starts a run. runJob answers 201 for a new run, 200 for the run in progress.
  listJobs(): Promise<ScheduledJob[]>                                          // GET  /jobs
  setJobEnabled(id: ID, enabled: boolean): Promise<ScheduledJob>               // PATCH /jobs/{id}  { enabled }
  runJob(id: ID): Promise<JobRun>                                              // POST /jobs/{id}/runs  (201, or 200 when already running)
  getRunLog(runId: ID): Promise<LogLine[]>                                     // GET  /runs/{id}/log  (last 2000 lines)

  // idle jobs (section 03 2.3). The definitions are idle-jobs.toml in the data folder.
  listIdleJobs(): Promise<IdleJob[]>                                           // GET  /idle-jobs
  setIdleJobEnabled(id: ID, enabled: boolean): Promise<IdleJob>                // PATCH /idle-jobs/{id}  { enabled }
  runIdleJob(id: ID): Promise<IdleJob>                                         // POST /idle-jobs/{id}/run  (ignores idle)
  getIdleJobLog(id: ID): Promise<LogLine[]>                                    // GET  /idle-jobs/{id}/log  (the newest run)
  getMachineState(): Promise<MachineState>                                     // GET  /machine

  // hardware: read at the moment of the call (no cache in the backend); only the Hardware screen calls it.
  listHardware(): Promise<HardwareTile[]>                                      // GET  /hardware
  openHardware(id: ID): Promise<void>                                          // POST /hardware/:id/open  (what the tile of the last read opens)

  // life: Rain reads the vault and writes one thing, the property a habit tick sets
  getVaultStatus(): Promise<VaultStatus>                                       // GET  /vault
  rereadVault(): Promise<void>                                                 // POST /vault/reread
  listHabitSuggestions(): Promise<PropertySuggestion[]>                        // GET  /vault/properties
  listHabits(): Promise<HabitSummary[]>                                        // GET  /habits
  saveHabit(draft: HabitDraft): Promise<Habit>                                 // POST /habits | PUT /habits/:id
  deleteHabit(id: ID): Promise<void>                                           // DELETE /habits/:id
  getHabitStats(id: ID): Promise<HabitStats>                                   // GET  /habits/:id/stats
  listHabitDays(from: ISODate, to: ISODate, habitId?: ID): Promise<HabitDay[]> // GET  /habit-days?from=&to=&habit=
  setHabitDone(id: ID, date: ISODate, done: boolean): Promise<HabitSummary>    // PUT  /habits/:id/days/:date  (body { done })
  listEvents(from: ISODate, to: ISODate): Promise<CalendarEvent[]>             // GET  /calendar/events?from=&to=  (always [] for now)
  openDailyNote(date: ISODate): Promise<OpenNoteResult>                        // POST /daily-notes/:date/open

  // disk
  listDrives(): Promise<Drive[]>                                               // GET  /drives
  startScan(driveId: ID): Promise<DiskScan>                                    // POST /scans            body { driveId }
  getScan(id: ID): Promise<DiskScan>                                           // GET  /scans/:id        `root` only when status is 'done'
  getScanNode(scanId: ID, path: string): Promise<DiskNode>                     // GET  /scans/:id/node?path=
  cancelScan(id: ID): Promise<void>                                            // DELETE /scans/:id      cancels a running scan, frees a finished one
  revealPath(path: string): Promise<void>                                      // POST /shell/reveal     body { path }
  checkPaths(paths: string[]): Promise<PathCheck[]>                            // POST /paths/check      body { paths }

  // game mode
  getGameMode(): Promise<GameModeState>                                        // GET  /game-mode
  startGameMode(opts?: { thenLaunchGameId?: ID }): Promise<GameModeState>      // POST /game-mode
  resolveGameMode(choice: GameModeChoice): Promise<GameModeState>              // POST /game-mode/choice
  markGameModeRecapSeen(): Promise<void>                                       // POST /game-mode/recap/seen

  // settings
  getSettings(): Promise<Settings>                                             // GET  /settings
  updateSettings(patch: SettingsPatch): Promise<Settings>                        // PATCH /settings
  getHealth(): Promise<Health>                                                  // GET  /health (config issues, source states)

  // live updates (one server-sent stream, GET /stream)
  subscribe<T extends Topic>(topic: T, cb: (payload: TopicPayload[T]) => void): () => void
}
