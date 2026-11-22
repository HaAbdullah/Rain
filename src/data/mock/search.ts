// Turns mock entities into SearchItems and ranks them for the launcher.
// The scoring is lib/score.ts; this file says which text and keywords each kind
// of item is found by, and ranks a title match above a keyword match.

import { score } from '@/lib/score'
import type {
  AppEntry, ClaudeSession, Game, GameSource, IdleJob, Pool, Project, ScheduledJob, SearchItem, SearchType, Status,
} from '../types'
import { image, lucide } from './engine'
import { findableGames, findableJobs, state } from './store'

const SOURCE_LABELS: Record<GameSource, string> = { steam: 'Steam', epic: 'Epic', custom: 'Custom' }

const IDLE_STATE_LABELS: Record<IdleJob['state'], string> = {
  queued: 'Queued', running: 'Running', paused: 'Paused', success: 'Done', failed: 'Failed',
}

// ---- entity -> SearchItem (score 0; search fills it in) ----

export function appItem(app: AppEntry): SearchItem {
  return { type: 'app', id: app.id, title: app.name, subtitle: app.exePath, icon: app.icon, score: 0 }
}

export function poolItem(pool: Pool): SearchItem {
  const count = pool.members.length
  return { type: 'pool', id: pool.id, title: pool.name, subtitle: `${count} ${count === 1 ? 'app' : 'apps'}`, icon: pool.icon, score: 0 }
}

export function projectItem(project: Project): SearchItem {
  return { type: 'project', id: project.id, title: project.name, subtitle: project.path, icon: project.icon, score: 0 }
}

export function gameItem(game: Game): SearchItem {
  const icon = game.coverUrl ? image(game.coverUrl) : lucide('gamepad-2')
  return { type: 'game', id: game.id, title: game.title, subtitle: SOURCE_LABELS[game.source], icon, score: 0 }
}

function sessionItem(session: ClaudeSession): SearchItem {
  const subtitle = session.request ? `${session.request.tool}: ${session.request.summary}` : folderName(session.cwd)
  return { type: 'session', id: session.id, title: session.title, subtitle, icon: lucide('bot'), status: session.status, score: 0 }
}

function jobItem(job: ScheduledJob): SearchItem {
  const status: Status = job.running ? 'running' : job.recentRuns[0]?.status ?? 'idle'
  return { type: 'job', id: job.id, title: job.name, subtitle: job.scheduleLabel, icon: lucide('calendar-clock'), status, score: 0 }
}

function idleJobItem(job: IdleJob): SearchItem {
  const status: Status = job.state === 'queued' || job.state === 'paused' ? 'idle' : job.state
  return { type: 'idleJob', id: job.id, title: job.name, subtitle: IDLE_STATE_LABELS[job.state], icon: lucide('cpu'), status, score: 0 }
}

function folderName(path: string): string {
  return path.slice(path.lastIndexOf('\\') + 1)
}

// ---- ranking ----

interface Candidate { item: SearchItem; keywords: string[] }

/** Everything that can be found, with the extra words each item answers to. */
function candidates(): Candidate[] {
  const appNames = new Map(state.apps.map((app) => [app.id, app.name]))
  return [
    ...state.apps.map((app) => ({ item: appItem(app), keywords: app.keywords })),
    // A pool is also found by the apps in it.
    ...state.pools.map((pool) => ({ item: poolItem(pool), keywords: pool.members.map((member) => appNames.get(member.appId) ?? '') })),
    ...state.projects.map((project) => ({ item: projectItem(project), keywords: [project.branch] })),
    ...findableGames().map((game) => ({ item: gameItem(game), keywords: [SOURCE_LABELS[game.source]] })),
    ...state.sessions.map((session) => ({ item: sessionItem(session), keywords: [folderName(session.cwd), session.branch ?? ''] })),
    ...findableJobs().map((job) => ({ item: jobItem(job), keywords: [job.scheduleLabel, job.kind] })),
    ...state.idleJobs.map((job) => ({ item: idleJobItem(job), keywords: [job.resource] })),
  ]
}

/**
 * A match that needs the keywords counts for this share of its score. Without
 * it "web" would list the browsers (keyword "web") above the pool "Web dev".
 */
const KEYWORD_SHARE = 0.6

function rank(query: string, { item, keywords }: Candidate): number {
  const byTitle = score(query, item.title)
  if (byTitle > 0) return byTitle
  const withKeywords = score(query, item.title, keywords)
  return withKeywords > 0 ? Math.max(1, Math.round(withKeywords * KEYWORD_SHARE)) : 0
}

/** Best match first. A blank query finds nothing: the Home list covers that case. */
export function searchItems(query: string, types?: SearchType[]): SearchItem[] {
  if (query.trim() === '') return []

  const results: SearchItem[] = []
  for (const candidate of candidates()) {
    if (types && !types.includes(candidate.item.type)) continue
    const points = rank(query, candidate)
    if (points > 0) results.push({ ...candidate.item, score: points })
  }
  // Equal scores keep a steady order, so the list does not jump between key presses.
  return results.sort((a, b) => b.score - a.score || a.title.localeCompare(b.title))
}
