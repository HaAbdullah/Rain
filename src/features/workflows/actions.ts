// Actions for Claude sessions, scheduled jobs and idle jobs (spec 4.3).
// On Home these rows only lead to Workflows: approving or stopping needs the
// exact command on screen, so those actions exist in the section only.
import { copyAction, hideAfterSuccess } from '@/actions/common'
import type { ActionContext, ActionProvider, RainAction, TargetOf } from '@/actions/types'
import { reactive } from 'vue'
import { refreshResources } from '@/composables/useResource'
import type { ClaudeSession, ID, IdleJob, PermissionAnswer, PermissionAnswerResult, ScheduledJob } from '@/data/types'
import { answerNotice } from './answerText'
import { focusLabel } from './sessions/sessionInfo'

type WorkflowsSub = 'sessions' | 'jobs' | 'idle'

/** Enter on a Home row: open Workflows on the right tab with this item selected. */
function showInWorkflows(sub: WorkflowsSub, id: ID): RainAction {
  return {
    id: 'show',
    label: 'Show in Workflows',
    icon: 'arrow-up-right',
    slot: 'primary',
    run: ({ nav }) => nav.go('workflows', sub, { id }),
  }
}

// ---- sessions ----

function transcript(session: ClaudeSession, slot: RainAction['slot'] = 'more'): RainAction {
  return {
    id: 'transcript',
    label: 'Transcript',
    icon: 'scroll-text',
    slot,
    keys: 'Ctrl+KeyL',
    run: ({ api, openLog }) =>
      openLog({ title: session.title, load: () => api.getSessionTranscript(session.id) }),
  }
}

/** Sessions whose answer is on its way. The list view moves its selection on at once. */
export const answering = reactive(new Set<ID>())

const toggling = new Set<ID>()

/** Runs a toggle unless one for the same job is in flight: a quick second press would flip from a stale snapshot. */
async function toggleOnce(id: ID, task: () => Promise<void>) {
  if (toggling.has(id)) return
  toggling.add(id)
  try {
    await task()
  } finally {
    toggling.delete(id)
  }
}

async function answer(session: ClaudeSession, choice: PermissionAnswer, ctx: ActionContext) {
  // The id of the request that is on screen: an answer for another one is refused by the backend,
  // so a request that changed between the render and the key press is never approved blind.
  const request = session.request
  if (!request || answering.has(session.id)) return
  answering.add(session.id)
  let result: PermissionAnswerResult
  try {
    result = await ctx.api.answerPermission(session.id, request.id, choice)
  } finally {
    answering.delete(session.id)
  }
  refreshResources('sessions')
  // The count is a courtesy: when it cannot be read, the answer still went through.
  const left = await ctx.api
    .listSessions()
    .then((sessions) => sessions.filter((other) => other.status === 'waiting').length)
    .catch(() => 0)
  ctx.notice.success(answerNotice(result, left, choice))
}

/** Only for a request that can still be answered from Rain. "Always allow" has no chord on purpose (spec 1.8). */
function permissionActions(session: ClaudeSession): RainAction[] {
  return [
    {
      id: 'allow-once',
      label: 'Allow once',
      icon: 'check',
      slot: 'more',
      keys: 'Ctrl+KeyY',
      run: (ctx) => answer(session, 'once', ctx),
    },
    {
      id: 'allow-always',
      label: 'Always allow',
      icon: 'check-check',
      slot: 'more',
      run: (ctx) => answer(session, 'always', ctx),
    },
    {
      id: 'deny',
      label: 'Deny',
      icon: 'ban',
      slot: 'more',
      keys: 'Ctrl+KeyD',
      run: (ctx) => answer(session, 'deny', ctx),
    },
  ]
}

function stopSession(session: ClaudeSession): RainAction {
  return {
    id: 'stop',
    label: 'Stop session',
    icon: 'circle-stop',
    slot: 'more',
    danger: true,
    async run({ api, confirm, notice }) {
      const confirmed = await confirm({
        title: `Stop "${session.title}"?`,
        body: 'The session ends now. Work it has not finished is lost.',
        confirmLabel: 'Stop session',
        danger: true,
      })
      if (!confirmed) return
      await api.stopSession(session.id)
      refreshResources('sessions')
      notice.success('Session stopped')
    },
  }
}

/** Only for a session whose folder is a known project: the backend opens projects, not loose folders. */
function openFolderInEditor(projectId: ID): RainAction {
  return {
    id: 'editor',
    label: 'Open folder in editor',
    icon: 'code-xml',
    slot: 'secondary',
    async run(ctx) {
      await ctx.launch({ type: 'project', id: projectId, mode: 'editorOnly' })
    },
  }
}

/** Brings the window of the session forward. `none` is not an error: Rain stays and says so. */
function focusSession(session: ClaudeSession): RainAction {
  return {
    id: 'focus',
    label: focusLabel(session.host),
    icon: 'square-terminal',
    slot: 'primary',
    async run(ctx) {
      const { focused } = await ctx.api.focusSession(session.id)
      if (focused === 'none') {
        ctx.notice.info('Could not find the window for this session')
        return
      }
      await hideAfterSuccess(ctx)
    },
  }
}

export const sessionActions: ActionProvider<TargetOf<'session'>> = ({ item: session }, { where }) => {
  if (where === 'home') return [showInWorkflows('sessions', session.id), transcript(session)]

  // An ended session has no window to focus, so Enter opens its transcript instead.
  const canStop = session.alive && session.host === 'terminal'
  return [
    session.alive ? focusSession(session) : transcript(session, 'primary'),
    ...(session.projectId ? [openFolderInEditor(session.projectId)] : []),
    ...(session.request?.answerable === true ? permissionActions(session) : []),
    ...(session.alive ? [transcript(session)] : []),
    copyAction('folder path', session.cwd),
    ...(canStop ? [stopSession(session)] : []),
  ]
}

// ---- scheduled jobs ----

/** The log viewer with the job's runs; the newest failed run is shown first, else the newest. */
function openJobLog(job: ScheduledJob, { api, openLog }: ActionContext) {
  const runs = job.recentRuns
  const shown = runs.find((run) => run.status === 'failed') ?? runs[0]
  openLog({
    title: job.name,
    runs,
    selectedRunId: shown?.id,
    load: (runId) => (runId ? api.getRunLog(runId) : Promise.resolve([])),
    // A run in progress is followed: the log is loaded again while the backend says it runs.
    follow: async (runId) =>
      runId !== undefined &&
      (await api.listJobs()).some(
        (j) => j.id === job.id && j.recentRuns.some((r) => r.id === runId && r.status === 'running'),
      ),
  })
}

function runJobNow(job: ScheduledJob): RainAction {
  return {
    id: 'run-now',
    label: 'Run now',
    icon: 'play',
    slot: 'more',
    keys: 'Ctrl+KeyR',
    async run({ api, notice }) {
      await api.runJob(job.id)
      refreshResources('jobs')
      notice.info(`${job.name} started`)
    },
  }
}

export const jobActions: ActionProvider<TargetOf<'job'>> = ({ item: job }, { where }) => {
  if (where === 'home') {
    return [
      showInWorkflows('jobs', job.id),
      {
        id: 'log',
        label: 'Log',
        icon: 'scroll-text',
        slot: 'more',
        keys: 'Ctrl+KeyL',
        run: (ctx) => openJobLog(job, ctx),
      },
      runJobNow(job),
    ]
  }

  return [
    {
      id: 'log',
      label: 'Open runs and log',
      icon: 'scroll-text',
      slot: 'primary',
      keys: 'Ctrl+KeyL',
      run: (ctx) => openJobLog(job, ctx),
    },
    {
      id: 'toggle',
      label: job.enabled ? 'Disable' : 'Enable',
      icon: job.enabled ? 'toggle-left' : 'toggle-right',
      slot: 'secondary',
      run: ({ api, notice }) =>
        toggleOnce(job.id, async () => {
          await api.setJobEnabled(job.id, !job.enabled)
          refreshResources('jobs')
          notice.success(job.enabled ? `${job.name} disabled` : `${job.name} enabled`)
        }),
    },
    runJobNow(job),
    copyAction('cron', job.cron),
  ]
}

// ---- idle jobs ----

function openIdleLog(job: IdleJob, slot: RainAction['slot'], label: string): RainAction {
  return {
    id: 'log',
    label,
    icon: 'scroll-text',
    slot,
    keys: 'Ctrl+KeyL',
    run: ({ api, openLog }) =>
      openLog({
        title: job.name,
        load: () => api.getIdleJobLog(job.id),
        // The log of a run in progress is followed.
        follow: async () => (await api.listIdleJobs()).some((j) => j.id === job.id && j.state === 'running'),
      }),
  }
}

function runIdleNow(job: IdleJob, label: string): RainAction {
  return {
    id: 'run-now',
    label,
    icon: 'play',
    slot: 'more',
    keys: 'Ctrl+KeyR',
    async run({ api, notice }) {
      await api.runIdleJob(job.id)
      refreshResources('idleJobs')
      notice.info(`${job.name} started`)
    },
  }
}

export const idleJobActions: ActionProvider<TargetOf<'idleJob'>> = ({ item: job }, { where }) => {
  if (where === 'home') {
    return [
      showInWorkflows('idle', job.id),
      openIdleLog(job, 'more', 'Log'),
      runIdleNow(job, 'Run now'),
    ]
  }

  return [
    openIdleLog(job, 'primary', 'Open log'),
    {
      id: 'toggle',
      label: job.enabled ? 'Disable' : 'Enable',
      icon: job.enabled ? 'toggle-left' : 'toggle-right',
      slot: 'secondary',
      run: ({ api, notice }) =>
        toggleOnce(job.id, async () => {
          await api.setIdleJobEnabled(job.id, !job.enabled)
          refreshResources('idleJobs')
          notice.success(job.enabled ? `${job.name} disabled` : `${job.name} enabled`)
        }),
    },
    runIdleNow(job, 'Run now (ignore idle)'),
  ]
}
