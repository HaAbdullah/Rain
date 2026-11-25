// What the Settings screen says about the state of the backend's side of a setting:
// the "Monitor Claude sessions" switch and the config files that could not be read.
import type { ConfigIssue, MonitorClaudeState, SourceStatus } from '@/data/types'
import type { StatusKind } from '@/lib/status'

/** A short line and a status glyph that replace the hint of a row. */
export interface RowState {
  status: StatusKind
  text: string
}

/** The state the backend reports for the Claude switch; `undefined` (off) leaves the plain hint. */
export function monitorClaudeRowState(state: MonitorClaudeState): RowState | undefined {
  switch (state) {
    case 'on':
      return { status: 'success', text: 'Active' }
    case 'onInactive':
      return { status: 'idle', text: 'Installed. Waiting for a session to start' }
    case 'needsRepair':
      return { status: 'failed', text: 'Needs repair. Press Ctrl+Enter' }
    case 'off':
      return undefined
  }
}

/** The source of Health.sources that each source row of Settings answers for. */
export const SOURCE_OF_ROW: Record<string, string> = {
  vault: 'vault',
  steam: 'steam',
  epic: 'epic',
  riot: 'riot',
  'game-folders': 'gamefolders',
}

/** What the backend says about a source row, in its own words; nothing when the source is fine or says nothing. */
export function sourceRowState(source: SourceStatus | undefined): RowState | undefined {
  if (!source || source.state === 'ok' || !source.message) return undefined
  return { status: source.state === 'failed' ? 'failed' : 'idle', text: source.message }
}

/** "settings.toml has an error on line 4": which file, and where when the parser said. */
export function configIssueText(issue: ConfigIssue): string {
  return issue.line === undefined
    ? `${issue.file} has an error`
    : `${issue.file} has an error on line ${issue.line}`
}
