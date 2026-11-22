// The mock's in-memory "database". Every slice reads and changes this one
// object; a reseed refills it from the seed files for the current scenario.

import type {
  AppEntry, CalendarEvent, ClaudeSession, DiskNode, DiskScan, Drive, Game, GameModeState, GameSource, Habit, HabitDay,
  ID, IdleJob, LaunchRun, MachineState, PinnableType, Pool, Project, ScheduledJob, Settings,
} from '../types'
import { cancelTimers } from './engine'
import { options } from './options'
import type { Scenario } from './options'
import { seed as seedApps } from './seed/apps'
import { seed as seedDrives } from './seed/disk'
import { seed as seedGameMode } from './seed/gameMode'
import { seed as seedGames } from './seed/games'
import { seed as seedLife } from './seed/life'
import { seed as seedProjects } from './seed/projects'
import { seed as seedSettings } from './seed/settings'
import { seed as seedWorkflows } from './seed/workflows'
import type { Schedule } from './seed/workflows'

export interface PinRef { type: PinnableType; id: ID }

export interface MockState {
  apps: AppEntry[]
  pools: Pool[]
  /** Apps whose executable is "missing": launching them always fails. */
  brokenAppIds: ID[]
  /** Pinned items in pin order. Each entity's own `pinned` flag is kept in step. */
  pins: PinRef[]
  projects: Project[]
  games: Game[]
  /** How many more times the Epic source fails before it loads. */
  epicFailuresLeft: number

  sessions: ClaudeSession[]
  quietSessionIds: ID[]
  jobs: ScheduledJob[]
  schedules: Record<ID, Schedule>
  idleJobs: IdleJob[]
  failingIdleJobIds: ID[]
  machine: MachineState

  /** Where the vault "exists"; undefined when there is none to find. */
  vaultPath: string | undefined
  missingProperties: string[]
  habits: Habit[]
  habitDays: HabitDay[]
  events: CalendarEvent[]

  drives: Drive[]
  scans: Record<ID, DiskScan>
  /** Finished trees by drive id, so a rescan does not rebuild them. */
  trees: Record<ID, DiskNode>

  launches: Record<ID, LaunchRun>
  gameMode: GameModeState
  settings: Settings
  /** Source of new ids (launches, scans, runs). */
  sequence: number
}

function build(scenario: Scenario): MockState {
  const { apps, pools, brokenAppIds } = seedApps(scenario)
  const projects = seedProjects(scenario)
  const games = seedGames(scenario)
  const workflows = seedWorkflows(scenario)
  const life = seedLife(scenario)

  for (const project of projects) {
    project.sessionIds = workflows.sessions.filter((session) => session.projectId === project.id).map((session) => session.id)
  }

  return {
    apps,
    pools,
    brokenAppIds,
    pins: [
      ...pinsOf('pool', pools),
      ...pinsOf('app', apps),
      ...pinsOf('project', projects),
      ...pinsOf('game', games),
    ],
    projects,
    games,
    // Typical shows one recoverable error: Epic fails on the first load only.
    epicFailuresLeft: scenario === 'typical' ? 1 : 0,
    ...workflows,
    ...life,
    drives: seedDrives(scenario),
    scans: {},
    trees: {},
    launches: {},
    gameMode: seedGameMode(scenario),
    settings: seedSettings(scenario),
    sequence: 0,
  }
}

function pinsOf(type: PinnableType, items: { id: ID; pinned: boolean }[]): PinRef[] {
  return items.filter((item) => item.pinned).map((item) => ({ type, id: item.id }))
}

/** The one state object. It is refilled in place, so slices can import it directly. */
export const state: MockState = build(options.scenario)

const resetHooks: (() => void)[] = []

/** Runs `hook` after every reseed. Slices use it to restart their background tickers. */
export function onStateReset(hook: () => void): void {
  resetHooks.push(hook)
}

/** Throws away every change and reseeds for the current scenario. */
export function resetState(): void {
  cancelTimers()
  Object.assign(state, build(options.scenario))
  for (const hook of resetHooks) hook()
}

/** A new id such as 'launch-7'. */
export function nextId(prefix: string): ID {
  state.sequence += 1
  return `${prefix}-${state.sequence}`
}

// ---- what can be found right now ----
// Search, pins and recents use these, so they never offer an item that its own
// list call would not return (the UI could not open such a result).

/** Custom games always show; a store's games show while that store is switched on in Settings. */
export function sourceEnabled(source: GameSource): boolean {
  if (source === 'steam') return state.settings.steamEnabled
  if (source === 'epic') return state.settings.epicEnabled
  return true
}

/** No game while the libraries are down (Failing), and never a hidden one. */
export function findableGames(): Game[] {
  if (options.scenario === 'failing') return []
  return state.games.filter((game) => !game.hidden && sourceEnabled(game.source))
}

/** No job while the scheduler is down (Failing). */
export function findableJobs(): ScheduledJob[] {
  return options.scenario === 'failing' ? [] : state.jobs
}
