// Launching an app, pool, project or game (spec 2.4). One place applies the
// rules: the window is held open while the checklist runs, Rain hides after a
// success unless it is kept open, and a failure stays on screen with a notice
// that names the failed step.
import { ref, type Ref } from 'vue'
import { api, onStreamReset } from '@/data'
import type { ID, LaunchRun, LaunchTarget } from '@/data/types'
import { errorMessage } from '@/lib/errors'
import { shell } from '@/shell'
import { useNotice } from './useNotice'
import { refreshResources } from './useResource'

/** The newest run per launched item (keyed by the target's id), for rows and checklists. */
const runs = ref<Record<ID, LaunchRun>>({})
/** Callers waiting for a run to finish, keyed by run id. */
const waiting = new Map<ID, { run: LaunchRun; resolve: (run: LaunchRun) => void }>()
let subscribed = false

/** A run that nobody reports as ended is asked about after this long, then every `POLL_EVERY_MS`. */
const POLL_AFTER_MS = 5000
const POLL_EVERY_MS = 3000
let pollTimer: number | undefined

/** The lists a launch changes (running state, "open", "last launched"), besides 'recent'. */
const CHANGED_BY: Record<LaunchTarget['type'], string[]> = {
  app: ['apps'],
  pool: ['pools', 'apps'],
  project: ['projects'],
  game: ['games'],
}

function isNotFound(reason: unknown): boolean {
  return typeof reason === 'object' && reason !== null && (reason as { code?: unknown }).code === 'notFound'
}

/** Asks the backend what became of every run we still wait for: a final frame may have been lost. */
async function reconcile(): Promise<void> {
  for (const [id, entry] of [...waiting]) {
    try {
      const current = await api.getLaunch(id)
      if (current.status === 'running') continue
      runs.value[current.target.id] = current
      entry.resolve(current)
      waiting.delete(id)
    } catch (reason) {
      // The backend does not know the run (it restarted), so it will never end.
      if (isNotFound(reason)) {
        entry.resolve({ ...entry.run, status: 'failed' })
        waiting.delete(id)
      }
    }
  }
}

/** Polls only while a run is waited for, so nothing runs when nothing launches. */
function schedulePoll(delay: number): void {
  if (pollTimer !== undefined || waiting.size === 0) return
  pollTimer = window.setTimeout(async () => {
    pollTimer = undefined
    await reconcile()
    schedulePoll(POLL_EVERY_MS)
  }, delay)
}

function follow() {
  if (subscribed) return
  subscribed = true
  api.subscribe('launch', (run) => {
    runs.value[run.target.id] = run
    if (run.status === 'running') return
    waiting.get(run.id)?.resolve(run)
    waiting.delete(run.id)
  })
  // A stream that reconnected may have lost a final frame: ask at once.
  onStreamReset(() => {
    void reconcile()
  })
}

/** Resolves when the run has ended. The api may answer with a run that is still going. */
function finished(run: LaunchRun): Promise<LaunchRun> {
  const seen = runs.value[run.target.id]
  // The last step may have been pushed before the launch call returned.
  const newest = seen?.id === run.id ? seen : run
  if (newest.status !== 'running') return Promise.resolve(newest)
  return new Promise((resolve) => {
    // The same run can be asked for twice (the backend hands back a run that is still going):
    // both callers are told when it ends, and both release their hold on the window.
    const earlier = waiting.get(run.id)
    waiting.set(run.id, {
      run: newest,
      resolve: earlier
        ? (ended) => {
            earlier.resolve(ended)
            resolve(ended)
          }
        : resolve,
    })
    schedulePoll(POLL_AFTER_MS)
  })
}

/**
 * Forgets every run. For a data reseed: the old runs will never report an end, so the
 * callers waiting for one are told "failed" and release their hold on the window.
 */
export function resetLaunches(): void {
  for (const { run, resolve } of waiting.values()) resolve({ ...run, status: 'failed' })
  waiting.clear()
  window.clearTimeout(pollTimer)
  pollTimer = undefined
  runs.value = {}
}

function failureText(run: LaunchRun, fallback: string): string {
  const step = run.steps.find((candidate) => candidate.status === 'failed')
  if (!step) return fallback
  return step.error ? `${step.label}: ${step.error}` : `${step.label} failed`
}

/**
 * Launches the target and resolves when every step has ended; it never rejects.
 * `name` is only used in the notice ("Launched Web dev").
 */
async function launch(target: LaunchTarget, name?: string): Promise<LaunchRun> {
  const notice = useNotice()
  const release = shell.holdOpen('launch')
  follow()

  let run: LaunchRun
  try {
    const started = await api.launch(target)
    // Record the run at once so panes and Enter guards see it before the first event.
    if (runs.value[target.id]?.id !== started.id) runs.value[target.id] = started
    run = await finished(started)
  } catch (reason) {
    // The launch could not even start (source unavailable). Report it as a failed run.
    run = { id: `launch-rejected-${target.id}`, target, status: 'failed', steps: [] }
    runs.value[target.id] = run
    notice.error(errorMessage(reason, 'Launch failed'))
    return run
  } finally {
    release()
  }

  runs.value[target.id] = run
  refreshResources('recent', ...CHANGED_BY[target.type])

  if (run.status === 'failed') {
    notice.error(failureText(run, name ? `Could not launch ${name}` : 'Launch failed'))
    return run
  }

  notice.success(name ? `Launched ${name}` : 'Launched')
  if (!shell.keepOpen.value) await shell.hide()
  return run
}

export interface Launcher {
  runs: Ref<Record<ID, LaunchRun>>
  launch(target: LaunchTarget, name?: string): Promise<LaunchRun>
}

const launcher: Launcher = { runs, launch }

export function useLaunch(): Launcher {
  return launcher
}
