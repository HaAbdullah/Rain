// The app-wide status vocabulary (spec 5.4). A status is always shown as a
// shape plus a colour; StatusDot owns the shapes, this file owns the colours.

import type { Status } from '@/data/types'

/** The five data states plus the three that only the UI derives. */
export type StatusKind = Status | 'stale' | 'paused' | 'queued'

/** CSS colour (a token reference) for each status. */
export const statusColor: Record<StatusKind, string> = {
  idle: 'var(--status-idle)',
  running: 'var(--status-running)',
  waiting: 'var(--status-waiting)',
  success: 'var(--status-success)',
  failed: 'var(--status-failed)',
  stale: 'var(--status-idle)',
  paused: 'var(--status-idle)',
  queued: 'var(--status-idle)',
}
