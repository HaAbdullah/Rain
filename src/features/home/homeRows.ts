// The rows Home draws, and the small functions that turn sessions, jobs, projects,
// commands and search items into them. Pure: no resources and no state live here.
import type { CommandDef } from '@/actions/commands'
import type { ActionTarget } from '@/actions/types'
import type {
  AppEntry,
  ClaudeSession,
  IconRef,
  ID,
  Pool,
  Project,
  ScheduledJob,
  SearchItem,
  SearchType,
  Status,
} from '@/data/types'
import { formatClock } from '@/lib/time'

export interface HomeRow {
  /** Unique in the list: ids of different types may collide, so it carries the type. */
  id: string
  title: string
  /** Drawn in the launcher list: only Needs you rows and projects have one (spec 7.1). */
  subtitle?: string
  /** What tells two search results with the same title apart (a path, a source). */
  disambiguator?: string
  icon: IconRef
  /** Lucide name of the small type icon at the end of the row. */
  typeIcon: string
  /** Replaces the type icon: a pool shows its members. */
  stack?: IconRef[]
  status?: Status
  target: ActionTarget
  /** A "Needs you" row: it has no Alt digit and is not where the cursor starts. */
  needsYou?: boolean
}

const TYPE_ICONS: Record<SearchType | 'command', string> = {
  app: 'app-window',
  pool: 'layers',
  project: 'folder-git-2',
  game: 'gamepad-2',
  session: 'bot',
  job: 'calendar-clock',
  idleJob: 'cpu',
  command: 'square-terminal',
}

function lucide(name: string): IconRef {
  return { kind: 'lucide', value: name }
}

function stackOf(pool: Pool, apps: AppEntry[]): IconRef[] {
  const iconById = new Map<ID, IconRef>(apps.map((app) => [app.id, app.icon]))
  return pool.members.flatMap((member) => iconById.get(member.appId) ?? [])
}

/** A row for a search, pinned or recent item whose entity has been found. */
export function itemRow(item: SearchItem, target: ActionTarget, apps: AppEntry[]): HomeRow {
  return {
    id: `${item.type}:${item.id}`,
    title: item.title,
    subtitle: target.type === 'project' ? target.item.branch : undefined,
    disambiguator: item.subtitle,
    icon: item.icon,
    typeIcon: TYPE_ICONS[item.type],
    stack: target.type === 'pool' ? stackOf(target.item, apps) : undefined,
    status: item.status,
    target,
  }
}

export function sessionRow(session: ClaudeSession): HomeRow {
  const request = session.request
  return {
    id: `session:${session.id}`,
    title: session.title,
    subtitle: request ? `${request.tool}: ${request.summary}` : session.lastMessage,
    icon: lucide(TYPE_ICONS.session),
    typeIcon: TYPE_ICONS.session,
    status: session.status,
    target: { type: 'session', id: session.id, item: session },
    needsYou: true,
  }
}

export function failedJobRow(job: ScheduledJob): HomeRow {
  const run = job.recentRuns[0]
  const exit = run?.exitCode === undefined ? '' : `, exit ${run.exitCode}`
  return {
    id: `job:${job.id}`,
    title: job.name,
    subtitle: run ? `Failed ${formatClock(run.startedAt)}${exit}` : 'Failed',
    icon: lucide(TYPE_ICONS.job),
    typeIcon: TYPE_ICONS.job,
    status: 'failed',
    target: { type: 'job', id: job.id, item: job },
    needsYou: true,
  }
}

export function projectRow(project: Project): HomeRow {
  return {
    id: `project:${project.id}`,
    title: project.name,
    subtitle: project.branch,
    icon: project.icon,
    typeIcon: TYPE_ICONS.project,
    target: { type: 'project', id: project.id, item: project },
  }
}

export function commandRow(command: CommandDef): HomeRow {
  return {
    id: `command:${command.id}`,
    title: command.title,
    icon: lucide(command.icon),
    typeIcon: TYPE_ICONS.command,
    target: { type: 'command', id: command.id, item: command },
  }
}
