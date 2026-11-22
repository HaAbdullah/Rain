// Generated log text for job runs, idle jobs and session transcripts. Each log
// is built from the item's id, so opening it twice shows the same lines.
// A failed run always ends with error lines that explain the failure.

import { formatDuration } from '@/lib/time'
import type { ClaudeSession, ID, IdleJob, JobRun, LogLine, ScheduledJob, SessionEvent } from '../types'
import { MINUTE, SECOND, createRng, slug } from './engine'
import type { Rng } from './engine'
import { sampleActivity } from './seed/workflows'

type Entry = Pick<LogLine, 'level' | 'text'>

const info = (text: string): Entry => ({ level: 'info', text })
const warn = (text: string): Entry => ({ level: 'warn', text })
const error = (text: string): Entry => ({ level: 'error', text })

// ---- why things fail ----

const JOB_FAILURES: Record<ID, string[]> = {
  'job-nightly-backup': [
    'Destination D:\\Backups is not reachable',
    'Retried 3 times, giving up',
    'Backup aborted: 0 of 1,284 files copied',
  ],
}
const DEFAULT_JOB_FAILURE = ['The last step returned an unexpected result', 'Stopping before any further changes are made']

const IDLE_FAILURES: Record<ID, string[]> = {
  'idle-thumbnail-cache': [
    'Cannot write to C:\\ProgramData\\Rain\\thumbnails: the folder is read-only',
    'Thumbnail cache stopped at 60%',
  ],
}
const DEFAULT_IDLE_FAILURE = ['The worker stopped unexpectedly']

// ---- job runs ----

const FOLDERS = ['C:\\Dev', 'C:\\Users\\Public\\Documents', 'D:\\Vault', 'C:\\ProgramData\\Rain\\cache', 'D:\\Media\\Photos']
const FILES = ['src/main.ts', 'package.json', 'README.md', 'src/lib/config.ts', 'docs/changelog.md', 'src/app.ts']

const CRON_STEPS: ((step: number, rng: Rng) => Entry)[] = [
  (step, rng) => info(`Processed batch ${step} (${rng.int(40, 900)} items)`),
  (_step, rng) => info(`Checked ${rng.pick(FOLDERS)}`),
  (_step, rng) => info(`Skipped ${rng.int(1, 300)} unchanged files`),
  (_step, rng) => info(`Wrote ${rng.int(4, 900)} KB to the cache`),
  (_step, rng) => warn(`Slow response from disk (${rng.int(2, 9)} s), continuing`),
]

const CLAUDE_STEPS: ((step: number, rng: Rng) => Entry)[] = [
  (_step, rng) => info(`Read ${rng.pick(FILES)}`),
  (_step, rng) => info(`Grep "${rng.pick(['TODO', 'deprecated', 'version', 'fixme'])}" (${rng.int(0, 40)} matches)`),
  (_step, rng) => info(`Bash: ${rng.pick(['git log --oneline -20', 'npm outdated', 'git diff --stat'])}`),
  (step) => info(`Drafted section ${step}`),
  (_step, rng) => warn(`Skipped ${rng.pick(FILES)}: file is larger than the read limit`),
]

/** The log of one job run: 30 to 60 lines for a finished run, fewer while it still runs. */
export function runLog(job: ScheduledJob, run: JobRun): LogLine[] {
  const rng = createRng(`log:${run.id}`)
  const steps = job.kind === 'claude' ? CLAUDE_STEPS : CRON_STEPS
  const ending = runEnding(job, run)
  const length = rng.int(30, 60)

  const entries: Entry[] = [
    info(`Starting "${job.name}" (${job.cron})`),
    info(job.kind === 'claude' ? 'Claude session started' : `Loaded C:\\ProgramData\\Rain\\jobs\\${slug(job.name)}.toml`),
  ]
  for (let step = 1; entries.length < length - ending.length; step += 1) {
    // Warnings are the last entry of each list and stay rare.
    const pickWarning = rng.chance(0.06)
    entries.push(steps[pickWarning ? steps.length - 1 : rng.int(0, steps.length - 2)](step, rng))
  }

  if (run.status === 'running') {
    // Still running: show the part that has happened so far, with no ending.
    const elapsed = Date.now() - Date.parse(run.startedAt)
    return stamp(entries.slice(0, Math.ceil(entries.length * 0.4)), run.startedAt, elapsed)
  }
  return stamp([...entries, ...ending], run.startedAt, run.durationMs ?? MINUTE)
}

function runEnding(job: ScheduledJob, run: JobRun): Entry[] {
  if (run.status !== 'failed') {
    return [info(`Finished in ${formatDuration(run.durationMs ?? 0)}`), info('Process exited with code 0')]
  }
  return [
    warn('A step failed, collecting details'),
    ...(JOB_FAILURES[job.id] ?? DEFAULT_JOB_FAILURE).map(error),
    error(`Process exited with code ${run.exitCode ?? 1}`),
  ]
}

// ---- idle jobs ----

const IDLE_CHUNKS = 24

/** The log of an idle job's latest run, up to the point its progress has reached. */
export function idleJobLog(job: IdleJob): LogLine[] {
  const rng = createRng(`log:${job.id}`)
  const entries: Entry[] = [info(`Started "${job.name}" after ${job.idleAfterMinutes} min idle`), info(`Using the ${job.resource.toUpperCase()}`)]

  // A queued job shows its previous, complete run.
  const reached = job.state === 'queued' ? IDLE_CHUNKS : Math.round(job.progress * IDLE_CHUNKS)
  for (let chunk = 1; chunk <= reached; chunk += 1) {
    entries.push(info(`Chunk ${chunk} of ${IDLE_CHUNKS} done (${rng.int(200, 4000)} items)`))
  }
  entries.push(...idleEnding(job))

  const startedAt = job.lastRunAt ?? new Date(Date.now() - 10 * MINUTE).toISOString()
  return stamp(entries, startedAt, entries.length * 20 * SECOND)
}

function idleEnding(job: IdleJob): Entry[] {
  switch (job.state) {
    case 'queued': return [info('Finished'), info(`Queued: starts again after ${job.idleAfterMinutes} min idle`)]
    case 'paused': return [warn(job.enabled ? 'Paused: the PC is in use' : 'Paused: the job is switched off')]
    case 'failed': return (IDLE_FAILURES[job.id] ?? DEFAULT_IDLE_FAILURE).map(error)
    case 'success': return [info('Finished')]
    case 'running': return []
  }
}

// ---- session transcripts ----

const EVENT_LABELS: Record<SessionEvent['kind'], string> = {
  prompt: 'You',
  tool: 'Tool',
  edit: 'Edit',
  message: 'Claude',
  permission: 'Permission',
  error: 'Error',
}

const TOOL_RESULTS = ['42 lines read', '3 matches in 2 files', 'exit 0', '118 lines read', 'no matches', '1 file changed']

/** A transcript: generated earlier history, then the session's recent events, then how it stands now. */
export function sessionTranscript(session: ClaudeSession): LogLine[] {
  const rng = createRng(`log:${session.id}`)
  const recent = [...session.recent].reverse()
  const lines: LogLine[] = [{ ts: session.startedAt, level: 'info', text: `Session started in ${session.cwd} (model ${session.model})` }]

  // Earlier work, spread between the start and the oldest recent event.
  const from = Date.parse(session.startedAt)
  const until = recent.length > 0 ? Date.parse(recent[0].at) : Date.parse(session.lastActivityAt)
  const earlier = rng.int(10, 20)
  for (let i = 1; i <= earlier; i += 1) {
    const at = new Date(from + ((until - from) * i) / (earlier + 1)).toISOString()
    lines.push(...eventLines({ at, ...sampleActivity(rng) }, rng))
  }
  for (const event of recent) lines.push(...eventLines(event, rng))

  lines.push({ ts: session.lastActivityAt, ...sessionEnding(session) })
  return lines
}

/** One event as a transcript line, with a result line under tool calls and edits. */
function eventLines(event: SessionEvent, rng: Rng): LogLine[] {
  const level: LogLine['level'] = event.kind === 'error' ? 'error' : event.kind === 'permission' ? 'warn' : 'info'
  const lines: LogLine[] = [{ ts: event.at, level, text: `${EVENT_LABELS[event.kind]}: ${event.text}` }]
  if (event.kind === 'tool' || event.kind === 'edit') {
    lines.push({ ts: event.at, level: 'info', text: `  ${rng.pick(TOOL_RESULTS)}` })
  }
  return lines
}

function sessionEnding(session: ClaudeSession): Entry {
  switch (session.status) {
    case 'waiting': return warn(`Waiting for your answer: ${session.request?.tool ?? 'tool'} ${session.request?.summary ?? ''}`.trimEnd())
    case 'failed': return error('Session stopped after an error')
    case 'success': return info('Session finished')
    case 'idle': return info('Waiting for the next prompt')
    case 'running': return info('Working')
  }
}

// ---- shared ----

/** Gives the entries evenly spaced timestamps from `startedAt` over `spanMs`. */
function stamp(entries: Entry[], startedAt: string, spanMs: number): LogLine[] {
  const start = Date.parse(startedAt)
  const step = spanMs / Math.max(1, entries.length - 1)
  return entries.map((entry, i) => ({ ts: new Date(start + i * step).toISOString(), ...entry }))
}
