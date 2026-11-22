// Small facts about a session that the row and the pane both show.
import type { ClaudeSession } from '@/data/types'

/** A working session that has been silent this long is marked stale (spec 7.5). */
const STALE_AFTER_MS = 60_000

/** Minutes of silence for a stale session (running, silent for 60 s or more), else null. */
export function silentMinutes(session: ClaudeSession, now: number): number | null {
  if (session.status !== 'running') return null
  const silent = now - Date.parse(session.lastActivityAt)
  return silent >= STALE_AFTER_MS ? Math.floor(silent / 60_000) : null
}

/** The last segment of the session's folder: a path ending in recipe-box gives 'recipe-box'. */
export function folderName(cwd: string): string {
  return cwd.split(/[\\/]/).filter(Boolean).pop() ?? cwd
}
