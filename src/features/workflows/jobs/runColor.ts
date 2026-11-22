// The fill of a run square (the Jobs run strip and the run list of the pane).
import type { Status } from '@/data/types'

export function runFill(status: Status): string {
  if (status === 'success') return 'bg-status-success'
  if (status === 'failed') return 'bg-status-failed'
  if (status === 'running') return 'bg-status-running'
  return 'bg-surface-2'
}
