// Mock slice: launching apps, pools, projects and games. launch() answers at
// once with a running LaunchRun; the steps then finish one by one and each
// change is sent on the 'launch' topic.

import type { RainApi } from '../../api'
import type { EnvStep, Game, ID, LaunchRun, LaunchStep, LaunchTarget, Pool, Project } from '../../types'
import { after, clone, emit, fail, findOrFail, image, lucide, stepDuration } from '../engine'
import { options } from '../options'
import { nextId, state } from '../store'

/** One step of a launch, with what the mock needs to know to play it out. */
interface PlannedStep {
  label: string
  icon: LaunchStep['icon']
  /** The app this step starts; it is marked as running when the step succeeds. */
  appId?: ID
  /** Extra wait before the step (a pool member's delay). */
  waitMs?: number
  /** Set when the step is going to fail. */
  error?: string
  /** A failure that makes the rest pointless (the project folder is gone). */
  stopsRun?: boolean
}

const STORE_NAMES = { steam: 'Steam', epic: 'Epic', custom: 'the game folder' } as const

export const launchesSlice: Pick<RainApi, 'launch' | 'getLaunch'> = {
  async launch(target) {
    const plan = planFor(target)
    const run: LaunchRun = {
      id: nextId('launch'),
      // Copied field by field: the target may be a reactive object.
      target: { type: target.type, id: target.id, mode: target.mode },
      status: 'running',
      steps: plan.map((step, i) => ({ label: step.label, icon: step.icon, status: i === 0 ? 'running' : 'idle' })),
    }
    state.launches[run.id] = run
    playStep(run, plan, 0)
    return clone(run)
  },

  async getLaunch(id) {
    const run = state.launches[id]
    if (!run) throw fail('notFound', 'Launch not found')
    return clone(run)
  },
}

// ---- planning ----

function planFor(target: LaunchTarget): PlannedStep[] {
  const plan = stepsFor(target)
  if (plan.length === 0) throw fail('failed', 'Nothing to launch')
  for (const step of plan) {
    if (step.appId !== undefined && state.brokenAppIds.includes(step.appId)) step.error = 'Executable not found'
  }
  if (options.scenario === 'failing') {
    // Every launch fails on its second step, or on its only one.
    plan[Math.min(1, plan.length - 1)].error ??= 'Failed to start'
  }
  return plan
}

function stepsFor(target: LaunchTarget): PlannedStep[] {
  switch (target.type) {
    case 'app': return [appStep(target.id)]
    case 'pool': return poolSteps(findOrFail(state.pools, target.id, 'Pool'))
    case 'project': return projectSteps(findOrFail(state.projects, target.id, 'Project'), target.mode)
    case 'game': return [gameStep(findOrFail(state.games, target.id, 'Game'))]
  }
}

function appStep(appId: ID): PlannedStep {
  const app = findOrFail(state.apps, appId, 'App')
  return { label: app.name, icon: app.icon, appId }
}

function poolSteps(pool: Pool): PlannedStep[] {
  return pool.members.map((member) => {
    const app = state.apps.find((candidate) => candidate.id === member.appId)
    if (!app) return { label: 'Unknown app', icon: lucide('app-window'), error: 'App not found' }
    return { label: app.name, icon: app.icon, appId: app.id, waitMs: member.delayMs }
  })
}

function projectSteps(project: Project, mode: LaunchTarget['mode']): PlannedStep[] {
  const wanted = mode === 'editorOnly' ? project.environment.filter((step) => step.kind === 'editor') : project.environment
  const plan = wanted.map((step: EnvStep): PlannedStep => ({ label: step.label, icon: step.icon, appId: step.appId }))
  if (project.missing && plan.length > 0) {
    plan[0].error = `Folder not found: ${project.path}`
    plan[0].stopsRun = true
  }
  return plan
}

function gameStep(game: Game): PlannedStep {
  const icon = game.coverUrl ? image(game.coverUrl) : lucide('gamepad-2')
  // A game that is not installed opens its install page instead of starting.
  const label = game.installed ? game.title : `Install ${game.title} from ${STORE_NAMES[game.source]}`
  return { label, icon }
}

// ---- playing the steps ----

function playStep(run: LaunchRun, plan: PlannedStep[], index: number): void {
  const planned = plan[index]
  after((planned.waitMs ?? 0) + stepDuration(), () => {
    const step = run.steps[index]
    if (planned.error === undefined) {
      step.status = 'success'
      markAppRunning(planned.appId)
    } else {
      step.status = 'failed'
      step.error = planned.error
    }

    // A failed step does not stop the others (a pool still opens its other apps),
    // unless nothing after it can work.
    const isLast = index === plan.length - 1 || planned.stopsRun === true
    if (isLast) {
      finish(run)
    } else {
      run.steps[index + 1].status = 'running'
    }
    emit('launch', clone(run))
    if (!isLast) playStep(run, plan, index + 1)
  })
}

function markAppRunning(appId: ID | undefined): void {
  const app = state.apps.find((candidate) => candidate.id === appId)
  if (app) {
    app.running = true
    app.lastLaunchedAt = new Date().toISOString()
  }
}

function finish(run: LaunchRun): void {
  run.status = run.steps.some((step) => step.status === 'failed') ? 'failed' : 'success'
  if (run.status === 'success') recordUse(run.target)
}

/** A successful launch shows up in "Recent" and in the item's own state. */
function recordUse(target: LaunchTarget): void {
  const now = new Date().toISOString()
  switch (target.type) {
    case 'app':
      // Already recorded when its step succeeded.
      break
    case 'pool': {
      const pool = state.pools.find((candidate) => candidate.id === target.id)
      if (pool) pool.lastLaunchedAt = now
      break
    }
    case 'project': {
      const project = state.projects.find((candidate) => candidate.id === target.id)
      if (project) {
        project.open = true
        project.lastOpenedAt = now
      }
      break
    }
    case 'game':
      markGamePlayed(target.id)
      break
  }
}

/** Marks an installed game as running. Game mode uses it for "then launch <game>". */
export function markGamePlayed(gameId: ID): void {
  const game = state.games.find((candidate) => candidate.id === gameId)
  if (game?.installed) {
    game.running = true
    game.lastPlayedAt = new Date().toISOString()
  }
}
