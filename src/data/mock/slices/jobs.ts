// Mock slice: scheduled jobs, idle jobs and the machine's idle state.
// "Run now" really runs: the job shows as running for a few seconds and then
// gets a new run. Changes are sent on the 'jobs' and 'idleJobs' topics.

import type { RainApi } from '../../api'
import type { ID, IdleJob, JobRun, ScheduledJob } from '../../types'
import { SECOND, after, clone, delay, emit, every, fail, findOrFail } from '../engine'
import { idleJobLog, runLog } from '../logs'
import { options } from '../options'
import { nextRun, runId } from '../seed/workflows'
import { state } from '../store'

const MAX_RUNS = 20
/** How long a "Run now" takes in the mock. */
const RUN_NOW_MS = 3 * SECOND

const IDLE_TICK_MS = 1 * SECOND
const IDLE_STEP = 0.1
/** A full idle run would take this long on a real machine; used for the "min left" figure. */
const IDLE_FULL_RUN_MINUTES = 30
/** Idle jobs that always fail stop here. */
const IDLE_FAILS_AT = 0.6

type JobsSlice = Pick<RainApi,
  | 'listJobs' | 'setJobEnabled' | 'runJob' | 'getRunLog'
  | 'listIdleJobs' | 'setIdleJobEnabled' | 'runIdleJob' | 'getIdleJobLog' | 'getMachineState'>

export const jobsSlice: JobsSlice = {
  async listJobs() {
    await delay()
    if (options.scenario === 'failing') throw fail('unavailable', 'Scheduler unavailable', 'jobs')
    return clone(state.jobs)
  },

  async setJobEnabled(id, enabled) {
    await delay()
    const job = findOrFail(state.jobs, id, 'Job')
    job.enabled = enabled
    job.nextRunAt = enabled ? nextRun(state.schedules[id]) : undefined
    publishJobs()
    return clone(job)
  },

  async runJob(id) {
    await delay()
    const job = findOrFail(state.jobs, id, 'Job')
    // Already running: answer with that run instead of starting a second one.
    if (job.running) return clone(job.recentRuns[0])

    const previous: JobRun | undefined = job.recentRuns[0]
    const run: JobRun = { id: runId(job.id, sequenceOf(previous) + 1), jobId: job.id, status: 'running', startedAt: new Date().toISOString() }
    job.recentRuns = [run, ...job.recentRuns].slice(0, MAX_RUNS)
    job.running = true
    publishJobs()

    // Whatever broke the last run is still broken, so a failed job fails again.
    const exitCode = previous?.status === 'failed' ? previous.exitCode ?? 1 : 0
    after(RUN_NOW_MS, () => {
      // Game mode may have finished this run already; a second finish would overwrite it.
      if (run.status !== 'running') return
      finishRun(job, run, exitCode)
      publishJobs()
    })
    return clone(run)
  },

  async getRunLog(id) {
    await delay()
    for (const job of state.jobs) {
      const run = job.recentRuns.find((candidate) => candidate.id === id)
      if (run) return runLog(job, run)
    }
    throw fail('notFound', 'Run not found')
  },

  async listIdleJobs() {
    await delay()
    return clone(state.idleJobs)
  },

  async setIdleJobEnabled(id, enabled) {
    await delay()
    const job = findOrFail(state.idleJobs, id, 'Idle job')
    job.enabled = enabled
    if (!enabled && (job.state === 'running' || job.state === 'queued')) {
      stopIdleTicker(id)
      job.state = 'paused'
    } else if (enabled && job.state === 'paused' && job.progress === 0) {
      // Nothing done yet: it goes back in the queue. A half-done job stays paused until the PC is idle.
      job.state = 'queued'
    }
    publishIdleJobs()
    return clone(job)
  },

  async runIdleJob(id) {
    await delay()
    const job = findOrFail(state.idleJobs, id, 'Idle job')
    if (job.state !== 'running') startIdleRun(job)
    return clone(job)
  },

  async getIdleJobLog(id) {
    await delay()
    return idleJobLog(findOrFail(state.idleJobs, id, 'Idle job'))
  },

  async getMachineState() {
    await delay()
    return clone(state.machine)
  },
}

// ---- scheduled jobs ----

/** Game mode "Wait for them": every running job finishes successfully. */
export function finishRunningJobs(): void {
  const running = state.jobs.filter((job) => job.running)
  for (const job of running) finishRun(job, job.recentRuns[0], 0)
  if (running.length > 0) publishJobs()
}

function finishRun(job: ScheduledJob, run: JobRun, exitCode: number): void {
  run.status = exitCode === 0 ? 'success' : 'failed'
  run.exitCode = exitCode
  run.durationMs = Date.now() - Date.parse(run.startedAt)
  job.running = false
}

/** 'run-nightly-backup-20' -> 20. Run ids end in a counter, so the next run is one higher. */
function sequenceOf(run: JobRun | undefined): number {
  return run ? Number(run.id.slice(run.id.lastIndexOf('-') + 1)) : 0
}

function publishJobs(): void {
  emit('jobs', clone(state.jobs))
}

// ---- idle jobs ----

const idleTickers = new Map<ID, () => void>()

function startIdleRun(job: IdleJob): void {
  // A paused job continues where it stopped; a finished or failed one starts over.
  if (job.state !== 'paused') job.progress = 0
  job.state = 'running'
  job.etaMinutes = minutesLeft(job.progress)
  publishIdleJobs()

  stopIdleTicker(job.id)
  idleTickers.set(job.id, every(IDLE_TICK_MS, () => {
    job.progress = Math.min(1, Math.round((job.progress + IDLE_STEP) * 100) / 100)
    job.etaMinutes = minutesLeft(job.progress)

    const fails = state.failingIdleJobIds.includes(job.id) && job.progress >= IDLE_FAILS_AT
    if (fails || job.progress === 1) {
      job.state = fails ? 'failed' : 'success'
      job.etaMinutes = undefined
      job.lastRunAt = new Date().toISOString()
      stopIdleTicker(job.id)
    }
    publishIdleJobs()
  }))
}

function stopIdleTicker(id: ID): void {
  idleTickers.get(id)?.()
  idleTickers.delete(id)
}

function minutesLeft(progress: number): number {
  return Math.ceil((1 - progress) * IDLE_FULL_RUN_MINUTES)
}

function publishIdleJobs(): void {
  emit('idleJobs', clone(state.idleJobs))
}
