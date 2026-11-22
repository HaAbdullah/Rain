// Launching an app, pool, project or game (spec 2.4). One place applies the
// rules: the window is held open while the checklist runs, Rain hides after a
// success unless it is kept open, and a failure stays on screen with a notice
// that names the failed step.
import { ref, type Ref } from 'vue'
import { api } from '@/data'
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

/** The lists a launch changes (running state, "open", "last launched"), besides 'recent'. */
const CHANGED_BY: Record<LaunchTarget['type'], string[]> = {
  app: ['apps'],
  pool: ['pools', 'apps'],
  project: ['projects'],
  game: ['games'],
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
}

/** Resolves when the run has ended. The api may answer with a run that is still going. */
function finished(run: LaunchRun): Promise<LaunchRun> {
  const seen = runs.value[run.target.id]
  // The last step may have been pushed before the launch call returned.
  const newest = seen?.id === run.id ? seen : run
  if (newest.status !== 'running') return Promise.resolve(newest)
  return new Promise((resolve) => waiting.set(run.id, { run: newest, resolve }))
}

/**
 * Forgets every run. For a data reseed: the old runs will never report an end, so the
 * callers waiting for one are told "failed" and release their hold on the window.
 */
export function resetLaunches(): void {
  for (const { run, resolve } of waiting.values()) resolve({ ...run, status: 'failed' })
  waiting.clear()
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
