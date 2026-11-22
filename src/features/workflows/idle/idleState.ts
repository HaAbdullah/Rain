// How an idle job's state reads, for its row and its pane.
import type { IdleJob } from '@/data/types'
import { formatMinutes } from '@/lib/format'

/** "Running, 12 min left", "Queued", "Paused at 40%". A disabled job just says so. */
export function idleStateText(job: IdleJob): string {
  if (!job.enabled) return 'Disabled'
  switch (job.state) {
    case 'running':
      return job.etaMinutes === undefined ? 'Running' : `Running, ${formatMinutes(job.etaMinutes)} left`
    case 'queued':
      return 'Queued'
    case 'paused':
      return job.progress > 0 ? `Paused at ${Math.round(job.progress * 100)}%` : 'Paused'
    case 'success':
      return 'Done'
    case 'failed':
      return 'Failed'
  }
}
