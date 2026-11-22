// Sample Claude sessions, scheduled jobs and idle jobs.
// Typical: 7 sessions (2 waiting, 2 working of which 1 stale, 1 done, 1 idle, 1 failed),
// 8 jobs (1 failed, 1 running, 1 disabled) with 20 runs each, and 3 idle jobs.

import type { ClaudeSession, ID, IdleJob, ISODateTime, JobRun, MachineState, PermissionRequest, ScheduledJob, SessionEvent, Status } from '../../types'
import { DAY, HOUR, MINUTE, SECOND, ago, createRng, slug } from '../engine'
import type { Rng } from '../engine'
import type { Scenario } from '../options'

// ---- sessions ----

export interface RequestSample { tool: string; summary: string; detail: string }

interface SessionRow {
  title: string
  /** Folder under C:\Dev; also links the session to that project. */
  project: string
  branch: string
  model: string
  status: Status
  /** Ages in ms: when it started and when it last did something. */
  started: number
  active: number
  tokens: number
  lastMessage: string
  request?: RequestSample
  /** Working but silent: it shows as stale and gets no new activity. */
  quiet?: boolean
}

const GAUGE_DIFF = `--- src/charts/gauge.ts
+++ src/charts/gauge.ts
@@ -12,7 +12,9 @@ export function drawGauge(ctx, reading) {
   const max = reading.max ?? 50
-  const ratio = reading.mm / max
+  // Clamp so a cloudburst cannot draw outside the dial.
+  const ratio = Math.min(1, reading.mm / max)
   ctx.beginPath()
-  ctx.arc(cx, cy, r, START, START + ratio * SWEEP)
+  const end = START + ratio * SWEEP
+  ctx.arc(cx, cy, r, START, end)
   ctx.stroke()`

const SETUP_GUIDE = `+++ docs/setup.md
+# Setup
+
+1. Install the dependencies with \`npm install\`.
+2. Copy \`press.example.toml\` to \`press.toml\`.
+3. Run \`npm run build\` to render the sample book.`

const SESSIONS: SessionRow[] = [
  { title: 'Fix login redirect loop', project: 'recipe-box', branch: 'fix/login-redirect', model: 'sonnet', status: 'waiting',
    started: 50 * MINUTE, active: 2 * MINUTE, tokens: 48_200, lastMessage: 'The test runner is missing. I need to install it before I can verify the fix.',
    request: { tool: 'Bash', summary: 'npm install', detail: 'cd C:\\Dev\\recipe-box\nnpm install --save-dev vitest @vue/test-utils' } },
  { title: 'Add rain gauge chart', project: 'weather-station', branch: 'feat/rain-gauge', model: 'opus', status: 'waiting',
    started: 35 * MINUTE, active: 6 * MINUTE, tokens: 31_000, lastMessage: 'The dial overflows on heavy rain. I want to clamp the ratio in drawGauge.',
    request: { tool: 'Edit', summary: 'src/charts/gauge.ts', detail: GAUGE_DIFF } },
  { title: 'Refactor sensor polling', project: 'sensor-hub', branch: 'main', model: 'sonnet', status: 'running',
    started: 20 * MINUTE, active: 15 * SECOND, tokens: 22_400, lastMessage: 'Moving the retry logic into one poll loop.' },
  { title: 'Tidy station config loader', project: 'weather-station', branch: 'feat/rain-gauge', model: 'sonnet', status: 'running',
    started: 70 * MINUTE, active: 4 * MINUTE, tokens: 61_900, lastMessage: 'Running the full test suite.', quiet: true },
  { title: 'Update README badges', project: 'dotfiles', branch: 'main', model: 'haiku', status: 'success',
    started: 3 * HOUR, active: 2 * HOUR, tokens: 6_300, lastMessage: 'Done. The badges now point at the main branch.' },
  { title: 'Explore tile renderer', project: 'pixel-garden', branch: 'main', model: 'sonnet', status: 'idle',
    started: 6 * HOUR, active: 5 * HOUR, tokens: 14_800, lastMessage: 'The renderer draws each tile twice. Tell me if you want a fix.' },
  { title: 'Migrate database schema', project: 'budget-tracker', branch: 'feat/csv-import', model: 'opus', status: 'failed',
    started: 2 * HOUR, active: 1 * HOUR, tokens: 87_500, lastMessage: 'Migration 0007 failed: column "category_id" already exists.' },
]

const BUSY_SESSIONS: SessionRow[] = [
  { title: 'Publish map tiles', project: 'trail-notes', branch: 'draft/maps', model: 'sonnet', status: 'waiting',
    started: 25 * MINUTE, active: 1 * MINUTE, tokens: 19_700, lastMessage: 'The tiles are built. I am ready to push the branch.',
    request: { tool: 'Bash', summary: 'git push origin draft/maps', detail: 'cd C:\\Dev\\trail-notes\ngit push origin draft/maps' } },
  { title: 'Draft setup guide', project: 'markdown-press', branch: 'release/2.0', model: 'haiku', status: 'waiting',
    started: 15 * MINUTE, active: 3 * MINUTE, tokens: 8_900, lastMessage: 'I wrote a first setup guide and want to save it.',
    request: { tool: 'Write', summary: 'docs/setup.md', detail: SETUP_GUIDE } },
  { title: 'Speed up move generator', project: 'chess-clock', branch: 'main', model: 'opus', status: 'waiting',
    started: 90 * MINUTE, active: 9 * MINUTE, tokens: 54_100, lastMessage: 'The new generator is in place. I need to run the tests.',
    request: { tool: 'Bash', summary: 'cargo test', detail: 'cd C:\\Dev\\chess-clock\ncargo test --release' } },
  { title: 'Add energy widget', project: 'home-dashboard', branch: 'main', model: 'sonnet', status: 'running',
    started: 12 * MINUTE, active: 20 * SECOND, tokens: 11_200, lastMessage: 'Wiring the widget to the meter feed.' },
  { title: 'Sort sprite atlas', project: 'pixel-garden', branch: 'main', model: 'sonnet', status: 'running',
    started: 8 * MINUTE, active: 30 * SECOND, tokens: 7_600, lastMessage: 'Grouping sprites by size before packing.' },
  { title: 'Clean shell aliases', project: 'dotfiles', branch: 'main', model: 'haiku', status: 'success',
    started: 9 * HOUR, active: 8 * HOUR, tokens: 4_100, lastMessage: 'Done. Removed 14 unused aliases.' },
  { title: 'Review budget categories', project: 'budget-tracker', branch: 'feat/csv-import', model: 'sonnet', status: 'idle',
    started: 1 * DAY, active: 20 * HOUR, tokens: 12_300, lastMessage: 'Three categories overlap. Say which one should win.' },
]

/** Requests used when the prototype simulates a new permission request. */
export const REQUEST_SAMPLES: RequestSample[] = [
  { tool: 'Bash', summary: 'npm run build', detail: 'npm run build' },
  { tool: 'Bash', summary: 'git commit', detail: 'git add -A\ngit commit -m "Tidy the poll loop"' },
  { tool: 'Edit', summary: 'src/config.ts', detail: '--- src/config.ts\n+++ src/config.ts\n@@ -4,3 +4,3 @@\n-export const RETRIES = 3\n+export const RETRIES = 5' },
  { tool: 'Bash', summary: 'rm -r dist', detail: 'rm -r dist' },
]

const ACTIVITY: Pick<SessionEvent, 'kind' | 'text'>[] = [
  { kind: 'tool', text: 'Read src/main.ts' },
  { kind: 'tool', text: 'Grep "TODO" in src' },
  { kind: 'tool', text: 'Bash: npm test' },
  { kind: 'tool', text: 'Read package.json' },
  { kind: 'tool', text: 'Glob src/**/*.ts' },
  { kind: 'tool', text: 'Bash: git status' },
  { kind: 'edit', text: 'Edit src/app.ts' },
  { kind: 'edit', text: 'Edit src/lib/config.ts' },
  { kind: 'edit', text: 'Write tests/app.test.ts' },
  { kind: 'message', text: 'Found the cause, preparing a fix.' },
  { kind: 'message', text: 'Checking the remaining call sites.' },
  { kind: 'message', text: 'Tests pass locally.' },
]

/** One plausible thing a working session did. */
export function sampleActivity(rng: Rng): Pick<SessionEvent, 'kind' | 'text'> {
  return rng.pick(ACTIVITY)
}

const EVENTS_PER_SESSION = 8

function toSession(row: SessionRow, rng: Rng): ClaudeSession {
  const id = `ses-${slug(row.title)}`
  const request: PermissionRequest | undefined = row.request && { id: `req-${slug(row.title)}`, ...row.request, requestedAt: ago(row.active) }
  return {
    id,
    title: row.title,
    cwd: `C:\\Dev\\${row.project}`,
    projectId: `proj-${row.project}`,
    branch: row.branch,
    model: row.model,
    status: row.status,
    startedAt: ago(row.started),
    lastActivityAt: ago(row.active),
    tokens: row.tokens,
    lastMessage: row.lastMessage,
    request,
    recent: events(row, rng),
  }
}

/** Eight events, newest first: the latest reflects the status, the oldest is the prompt. */
function events(row: SessionRow, rng: Rng): SessionEvent[] {
  const latest: Pick<SessionEvent, 'kind' | 'text'> = row.request
    ? { kind: 'permission', text: `${row.request.tool}: ${row.request.summary}` }
    : { kind: row.status === 'failed' ? 'error' : 'message', text: row.lastMessage }

  const list: SessionEvent[] = []
  let age = row.active
  for (let i = 0; i < EVENTS_PER_SESSION; i += 1) {
    const isOldest = i === EVENTS_PER_SESSION - 1
    const what = i === 0 ? latest : isOldest ? { kind: 'prompt' as const, text: row.title } : sampleActivity(rng)
    list.push({ at: ago(age), ...what })
    age += rng.int(20, 180) * SECOND
  }
  return list
}

// ---- scheduled jobs ----

export interface Schedule {
  cron: string
  label: string
  periodMs: number
  minute: number
  hour?: number
  /** 0 = Sunday. */
  weekday?: number
}

const EVERY_30_MIN: Schedule = { cron: '*/30 * * * *', label: 'Every 30 min', periodMs: 30 * MINUTE, minute: 0 }
const HOURLY: Schedule = { cron: '15 * * * *', label: 'Hourly at :15', periodMs: HOUR, minute: 15 }
const DAILY_02: Schedule = { cron: '0 2 * * *', label: 'Daily 02:00', periodMs: DAY, minute: 0, hour: 2 }
const DAILY_06: Schedule = { cron: '0 6 * * *', label: 'Daily 06:00', periodMs: DAY, minute: 0, hour: 6 }
const DAILY_12: Schedule = { cron: '0 12 * * *', label: 'Daily 12:00', periodMs: DAY, minute: 0, hour: 12 }
const SUNDAYS: Schedule = { cron: '0 0 * * 0', label: 'Sundays 00:00', periodMs: 7 * DAY, minute: 0, hour: 0, weekday: 0 }
const MONDAYS: Schedule = { cron: '0 9 * * 1', label: 'Mondays 09:00', periodMs: 7 * DAY, minute: 0, hour: 9, weekday: 1 }
const FRIDAYS: Schedule = { cron: '0 18 * * 5', label: 'Fridays 18:00', periodMs: 7 * DAY, minute: 0, hour: 18, weekday: 5 }

interface JobRow {
  name: string
  kind: 'cron' | 'claude'
  schedule: Schedule
  /** A normal run takes about this long. */
  typicalMs: number
  /** failed: the latest run failed. running: a run is in progress. disabled: switched off. */
  state?: 'failed' | 'running' | 'disabled'
  /** One older run failed, so the run strip is not all green. */
  flaky?: boolean
}

const JOBS: JobRow[] = [
  { name: 'Nightly backup', kind: 'cron', schedule: DAILY_02, typicalMs: 4 * MINUTE, state: 'failed' },
  { name: 'Sync notes vault', kind: 'cron', schedule: EVERY_30_MIN, typicalMs: 20 * SECOND, state: 'running' },
  { name: 'Rotate logs', kind: 'cron', schedule: SUNDAYS, typicalMs: 8 * SECOND },
  { name: 'Update package index', kind: 'cron', schedule: HOURLY, typicalMs: 35 * SECOND, flaky: true },
  { name: 'Clean temp files', kind: 'cron', schedule: DAILY_12, typicalMs: 12 * SECOND, state: 'disabled' },
  { name: 'Morning brief', kind: 'claude', schedule: DAILY_06, typicalMs: 2 * MINUTE },
  { name: 'Dependency review', kind: 'claude', schedule: MONDAYS, typicalMs: 5 * MINUTE, flaky: true },
  { name: 'Changelog draft', kind: 'claude', schedule: FRIDAYS, typicalMs: 3 * MINUTE },
]

// Busy scenario: job names made from two word lists.
const JOB_VERBS = ['Sync', 'Check', 'Prune', 'Export', 'Index', 'Verify']
const JOB_SUBJECTS = ['photo archive', 'download folder', 'repo mirrors', 'font cache', 'DNS records', 'feed reader', 'certificates', 'container images']
const BUSY_SCHEDULES = [EVERY_30_MIN, HOURLY, DAILY_02, DAILY_06, DAILY_12, SUNDAYS, MONDAYS, FRIDAYS]
const BUSY_JOB_COUNT = 30

const RUNS_PER_JOB = 20

/** The most recent scheduled start at or before `now`. */
function lastOccurrence(schedule: Schedule, now: number): number {
  const d = new Date(now)
  d.setSeconds(0, 0)
  if (schedule.periodMs < HOUR) {
    const step = schedule.periodMs / MINUTE
    d.setMinutes(d.getMinutes() - (d.getMinutes() % step))
  } else {
    d.setMinutes(schedule.minute)
  }
  if (schedule.hour !== undefined) d.setHours(schedule.hour)
  if (schedule.weekday !== undefined) d.setDate(d.getDate() - ((d.getDay() - schedule.weekday + 7) % 7))
  return d.getTime() > now ? d.getTime() - schedule.periodMs : d.getTime()
}

/** When the job runs next. */
export function nextRun(schedule: Schedule): ISODateTime {
  return new Date(lastOccurrence(schedule, Date.now()) + schedule.periodMs).toISOString()
}

function toJob(row: JobRow, rng: Rng): ScheduledJob {
  const id = `job-${slug(row.name)}`
  return {
    id,
    name: row.name,
    kind: row.kind,
    cron: row.schedule.cron,
    scheduleLabel: row.schedule.label,
    enabled: row.state !== 'disabled',
    running: row.state === 'running',
    nextRunAt: row.state === 'disabled' ? undefined : nextRun(row.schedule),
    recentRuns: runs(id, row, rng),
  }
}

/** Twenty runs, newest first. */
function runs(jobId: ID, row: JobRow, rng: Rng): JobRun[] {
  const { periodMs } = row.schedule
  let start = lastOccurrence(row.schedule, Date.now())
  // A start in the last few minutes would still be running: begin one period earlier.
  if (Date.now() - start < 10 * MINUTE) start -= periodMs
  // A disabled job stopped running a while ago.
  if (row.state === 'disabled') start -= 5 * periodMs

  const list: JobRun[] = Array.from({ length: RUNS_PER_JOB }, (_, k) => ({
    id: runId(jobId, RUNS_PER_JOB - k),
    jobId,
    status: 'success',
    startedAt: new Date(start - k * periodMs).toISOString(),
    durationMs: Math.round(row.typicalMs * (0.7 + rng.next() * 0.7)),
    exitCode: 0,
  }))

  if (row.flaky) markFailed(list[rng.int(4, 9)], 2)
  if (row.state === 'failed') markFailed(list[0], 1)
  if (row.state === 'running') {
    list.pop()
    list.unshift({ id: runId(jobId, RUNS_PER_JOB + 1), jobId, status: 'running', startedAt: ago(45 * SECOND) })
  }
  return list
}

/** 'job-nightly-backup', 20 -> 'run-nightly-backup-20'. */
export function runId(jobId: ID, sequence: number): ID {
  return `${jobId.replace(/^job-/, 'run-')}-${sequence}`
}

function markFailed(run: JobRun, exitCode: number): void {
  run.status = 'failed'
  run.exitCode = exitCode
  // A failing run usually stops early.
  run.durationMs = Math.round((run.durationMs ?? 0) / 3)
}

function generatedJobs(rng: Rng, count: number): JobRow[] {
  const names = JOB_VERBS.flatMap((verb) => JOB_SUBJECTS.map((subject) => `${verb} ${subject}`))
  return rng.shuffle(names).slice(0, count).map((name) => ({
    name,
    kind: rng.chance(0.25) ? 'claude' : 'cron',
    schedule: rng.pick(BUSY_SCHEDULES),
    typicalMs: rng.int(5, 240) * SECOND,
    flaky: rng.chance(0.3),
  }))
}

// ---- idle jobs ----

interface IdleRow {
  name: string
  state: IdleJob['state']
  progress: number
  resource: IdleJob['resource']
  /** How long ago it last ran, in ms. */
  lastRun: number
  etaMinutes?: number
  disabled?: boolean
}

const IDLE_JOBS: IdleRow[] = [
  { name: 'Photo library index', state: 'queued', progress: 0, resource: 'gpu', lastRun: 2 * DAY },
  { name: 'Search index rebuild', state: 'paused', progress: 0.4, resource: 'cpu', lastRun: 3 * HOUR, etaMinutes: 18 },
  { name: 'Thumbnail cache', state: 'failed', progress: 0.6, resource: 'cpu', lastRun: 9 * HOUR },
]

const BUSY_IDLE_JOBS: IdleRow[] = [
  { name: 'Embedding refresh', state: 'queued', progress: 0, resource: 'gpu', lastRun: 1 * DAY },
  { name: 'Archive compaction', state: 'success', progress: 1, resource: 'cpu', lastRun: 6 * HOUR },
  { name: 'Duplicate finder', state: 'paused', progress: 0, resource: 'cpu', lastRun: 12 * DAY, disabled: true },
  { name: 'Font cache warm-up', state: 'success', progress: 1, resource: 'cpu', lastRun: 14 * HOUR },
  { name: 'Vault link check', state: 'paused', progress: 0.7, resource: 'cpu', lastRun: 4 * HOUR, etaMinutes: 6 },
]

const IDLE_AFTER_MINUTES = 10

function toIdleJob(row: IdleRow): IdleJob {
  return {
    id: `idle-${slug(row.name)}`,
    name: row.name,
    enabled: !row.disabled,
    state: row.state,
    progress: row.progress,
    resource: row.resource,
    idleAfterMinutes: IDLE_AFTER_MINUTES,
    etaMinutes: row.etaMinutes,
    lastRunAt: ago(row.lastRun),
  }
}

/** Idle jobs that fail again whenever they are run. */
const FAILING_IDLE_JOBS: ID[] = ['idle-thumbnail-cache']

// ---- seed ----

export interface WorkflowSeed {
  sessions: ClaudeSession[]
  /** Working sessions that stay silent (stale). */
  quietSessionIds: ID[]
  jobs: ScheduledJob[]
  /** How each job repeats, by job id. Needed to work out the next run when a job is enabled. */
  schedules: Record<ID, Schedule>
  idleJobs: IdleJob[]
  failingIdleJobIds: ID[]
  machine: MachineState
}

export function seed(scenario: Scenario): WorkflowSeed {
  const machine: MachineState = { userActive: true, idleMinutes: 0 }
  if (scenario === 'empty') {
    return { sessions: [], quietSessionIds: [], jobs: [], schedules: {}, idleJobs: [], failingIdleJobIds: [], machine }
  }

  const busy = scenario === 'busy'
  const rng = createRng('workflows')
  const sessionRows = busy ? [...SESSIONS, ...BUSY_SESSIONS] : SESSIONS
  const sessions = sessionRows.map((row) => toSession(row, rng))
  const jobRows = busy ? [...JOBS, ...generatedJobs(rng, BUSY_JOB_COUNT - JOBS.length)] : JOBS
  const jobs = jobRows.map((row) => toJob(row, rng))
  const idleJobs = (busy ? [...IDLE_JOBS, ...BUSY_IDLE_JOBS] : IDLE_JOBS).map(toIdleJob)

  return {
    sessions,
    quietSessionIds: sessions.filter((_, i) => sessionRows[i].quiet).map((session) => session.id),
    jobs,
    schedules: Object.fromEntries(jobs.map((job, i) => [job.id, jobRows[i].schedule])),
    idleJobs,
    failingIdleJobIds: FAILING_IDLE_JOBS,
    machine,
  }
}
