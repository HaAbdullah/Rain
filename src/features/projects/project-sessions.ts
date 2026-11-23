// Helpers that connect a project to its Claude sessions: which sessions live there
// and which single status its card shows.
import type { ClaudeSession, Project } from '@/data/types'
import type { StatusKind } from '@/lib/status'

/** A working session silent for this long counts as stale (spec 5.4). */
const STALE_AFTER_MS = 60_000

/** Most urgent first: what needs you comes before what is merely running. */
const URGENCY: StatusKind[] = ['waiting', 'running', 'stale', 'failed', 'success', 'idle']

export function sessionsOfProject(project: Project, sessions: ClaudeSession[]): ClaudeSession[] {
  return sessions.filter((session) => session.projectId === project.id)
}

export function sessionStatus(session: ClaudeSession, now: number): StatusKind {
  const silentFor = now - Date.parse(session.lastActivityAt)
  return session.status === 'running' && silentFor >= STALE_AFTER_MS ? 'stale' : session.status
}

/** The most urgent status among the sessions, or undefined when there are none. */
export function leadStatus(sessions: ClaudeSession[], now: number): StatusKind | undefined {
  const kinds = sessions.map((session) => sessionStatus(session, now))
  return URGENCY.find((kind) => kinds.includes(kind))
}
