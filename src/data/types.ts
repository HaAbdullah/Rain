// Every entity the UI reads. These shapes are the contract with the backend.
// The types the backend already owns are generated from Rust (src/bindings/rain-types.ts)
// and re-exported here, so screens keep importing from this one file. The types that are
// not ported yet stay hand-written below; a stage deletes its types from here when it
// ports them (scripts/check.mjs fails on a name that is in both places).

import type { IconRef, Status } from '../bindings/rain-types'

export * from '../bindings/rain-types'

// Hand-written aliases: the generator turns these into plain `string`.
export type ID = string
export type ISODate = string        // '2026-11-21'
export type ISODateTime = string    // '2026-11-21T09:30:00.000Z'

// ---- search ----
export type SearchType = 'app' | 'pool' | 'project' | 'game' | 'session' | 'job' | 'idleJob'
export type PinnableType = 'app' | 'pool' | 'project' | 'game'
/** `score` is on the scale of lib/score.ts: Home ranks items and commands together. */
export interface SearchItem { type: SearchType; id: ID; title: string; subtitle?: string; icon: IconRef; status?: Status; score: number }

// ---- apps and pools ----
export interface AppEntry { id: ID; name: string; icon: IconRef; exePath: string; keywords: string[];
  pinned: boolean; running: boolean; lastLaunchedAt?: ISODateTime }
export interface PoolMember { appId: ID; args?: string; delayMs?: number }
export interface Pool { id: ID; name: string; icon: IconRef; members: PoolMember[]; pinned: boolean; lastLaunchedAt?: ISODateTime }
export interface PoolDraft { id?: ID; name: string; members: PoolMember[] }

// ---- launching ----
export interface LaunchTarget { type: 'app' | 'pool' | 'project' | 'game'; id: ID; mode?: 'full' | 'editorOnly' }
export interface LaunchStep { label: string; icon: IconRef; status: Status; error?: string }
export interface LaunchRun { id: ID; target: LaunchTarget; status: 'running' | 'success' | 'failed'; steps: LaunchStep[] }

// ---- projects ----
export interface EnvStep { kind: 'editor' | 'terminal' | 'browser' | 'claude' | 'app'; label: string; icon: IconRef;
  appId?: ID; command?: string; url?: string }
export interface Project { id: ID; name: string; path: string; icon: IconRef; branch: string; dirtyFiles: number;
  commits14d: number[]; lastOpenedAt: ISODateTime; activityScore: number; environment: EnvStep[];
  sessionIds: ID[]; open: boolean; pinned: boolean; missing: boolean }
//   commits14d: one count per day, oldest first; the last entry is today

// ---- games ----
export type GameSource = 'steam' | 'epic' | 'custom'
export interface Game { id: ID; title: string; source: GameSource; coverUrl?: string; installed: boolean; running: boolean;
  sizeBytes?: number; playtimeMinutes: number; lastPlayedAt?: ISODateTime; pinned: boolean; hidden: boolean }
export interface GameLibrary { games: Game[]; failedSources: GameSource[] }
export interface CustomGameDraft { title: string; exePath: string; coverPath?: string }

// ---- Claude sessions ----
export type PermissionAnswer = 'once' | 'always' | 'deny'
export interface PermissionRequest { id: ID; tool: string; summary: string; detail: string; requestedAt: ISODateTime }
//   summary: one line for rows ('npm install'); detail: the exact command or diff for the pane
export interface SessionEvent { at: ISODateTime; kind: 'prompt' | 'tool' | 'edit' | 'message' | 'permission' | 'error'; text: string }
export interface ClaudeSession { id: ID; title: string; cwd: string; projectId?: ID; branch?: string; model: string;
  status: Status; startedAt: ISODateTime; lastActivityAt: ISODateTime; tokens: number; lastMessage: string;
  request?: PermissionRequest; recent: SessionEvent[] }
//   status: running = working, waiting = waiting on you, success = done, idle, failed
//   stale is derived in the UI: status === 'running' && now - lastActivityAt >= 60 s
//   recent: newest first, at most 8

// ---- scheduled and idle jobs ----
export interface JobRun { id: ID; jobId: ID; status: Status; startedAt: ISODateTime; durationMs?: number; exitCode?: number }
export interface LogLine { ts: ISODateTime; level: 'info' | 'warn' | 'error'; text: string }
export interface ScheduledJob { id: ID; name: string; kind: 'cron' | 'claude'; cron: string; scheduleLabel: string;
  enabled: boolean; running: boolean; nextRunAt?: ISODateTime; recentRuns: JobRun[] }   // newest first, at most 20
export interface IdleJob { id: ID; name: string; enabled: boolean; state: 'queued' | 'running' | 'paused' | 'success' | 'failed';
  progress: number; resource: 'cpu' | 'gpu'; idleAfterMinutes: number; etaMinutes?: number; lastRunAt?: ISODateTime }
export interface MachineState { userActive: boolean; idleMinutes: number }

// ---- life ----
export interface Habit { id: ID; name: string; icon: IconRef; property: string; kind: 'boolean' | 'number'; target?: number; unit?: string }
export type HabitValue = boolean | number | 'skip' | null          // null = note exists, property not set
export interface HabitDay { date: ISODate; noteExists: boolean; values: Record<ID, HabitValue> }   // one row per daily note
export interface VaultStatus { connected: boolean; path?: string; missingProperties: string[] }
export interface CalendarEvent { id: ID; title: string; start: ISODateTime; end: ISODateTime; allDay: boolean;
  calendar: string; color: string; location?: string }

// ---- disk ----
export type FileCategory = 'video' | 'image' | 'audio' | 'archive' | 'code' | 'document' | 'app' | 'other'
export interface Drive { id: ID; letter: string; label: string; totalBytes: number; usedBytes: number }
export interface DiskNode { name: string; path: string; sizeBytes: number; kind: 'dir' | 'file'; category?: FileCategory; children?: DiskNode[] }
export interface DiskScan { id: ID; driveId: ID; status: 'scanning' | 'done' | 'failed' | 'cancelled';
  progress: number; filesSeen: number; error?: string; root?: DiskNode }

// ---- game mode ----
export type GameModeChoice = 'wait' | 'stopAnyway' | 'closeNow' | 'cancel'
export interface WindDownItem { id: ID; label: string; icon: IconRef; kind: 'session' | 'job' | 'idleJob' | 'service' | 'window';
  count: number; effect: 'safe' | 'interrupts'; status: 'pending' | 'stopping' | 'stopped' }
export interface GameModeState { phase: 'off' | 'review' | 'countdown' | 'windingDown' | 'done';
  waiting: boolean; secondsLeft: number; countdownSeconds: number; items: WindDownItem[]; thenLaunchGameId?: ID }
