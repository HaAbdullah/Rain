// The one line that tells what the last game mode did (section 06 6.7.7 and 6.11).
import type { GameModeRecap } from '@/data/types'

const plural = (n: number, one: string, many: string) => `${n} ${n === 1 ? one : many}`

/** One line for the notice, at most about 90 characters. */
export function recapText(recap: GameModeRecap): string {
  if (!recap.complete) return 'Game mode did not finish.'
  const sum = (kinds: string[], outcomes: string[]) =>
    recap.items
      .filter((item) => kinds.includes(item.kind) && outcomes.includes(item.outcome))
      .reduce((total, item) => total + item.count, 0)
  const jobs = ['job', 'idleJob']
  const apps = ['otherApp']
  const stoppedJobs = sum(jobs, ['stopped', 'forced'])
  const closedApps = sum(apps, ['stopped', 'forced'])
  const left = sum(['session'], ['leftRunning'])

  const parts: string[] = []
  if (closedApps > 0 && stoppedJobs > 0) {
    parts.push(`Game mode closed ${plural(closedApps, 'app', 'apps')} and stopped ${plural(stoppedJobs, 'job', 'jobs')}.`)
  } else if (closedApps > 0) {
    parts.push(`Game mode closed ${plural(closedApps, 'app', 'apps')}.`)
  } else if (stoppedJobs > 0) {
    parts.push(`Game mode stopped ${plural(stoppedJobs, 'job', 'jobs')}.`)
  } else {
    parts.push('Game mode closed Rain.')
  }
  const failedJobs = sum(jobs, ['failed'])
  if (failedJobs > 0) parts.push(`${plural(failedJobs, 'job', 'jobs')} did not stop.`)
  // A program that still shows a window is asking the user something: it was left alone.
  const stayed = sum(apps, ['leftRunning'])
  if (stayed > 0) parts.push(`${plural(stayed, 'app', 'apps')} did not close.`)
  const refused = sum(apps, ['failed'])
  if (refused > 0) parts.push(`${plural(refused, 'app', 'apps')} need administrator rights.`)
  if (left > 0) parts.push(`${plural(left, 'Claude session', 'Claude sessions')} kept running.`)
  if (recap.launchError) parts.push('The game did not start.')
  else if (recap.launchedInJob === true) parts.push('The game may close with Rain.')
  return parts.join(' ')
}
