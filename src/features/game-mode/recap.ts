// The one line that tells what the last game mode did (section 06 6.7.7).
import type { GameModeRecap } from '@/data/types'

const plural = (n: number, one: string, many: string) => `${n} ${n === 1 ? one : many}`

/** One line for the notice, at most about 70 characters. */
export function recapText(recap: GameModeRecap): string {
  if (!recap.complete) return 'Game mode did not finish.'
  const sum = (outcomes: string[]) =>
    recap.items
      .filter((item) => item.kind !== 'session' && outcomes.includes(item.outcome))
      .reduce((total, item) => total + item.count, 0)
  const stopped = sum(['stopped', 'forced'])
  const left = recap.items
    .filter((item) => item.outcome === 'leftRunning')
    .reduce((total, item) => total + item.count, 0)
  const parts = [stopped > 0 ? `Game mode stopped ${plural(stopped, 'job', 'jobs')}.` : 'Game mode closed Rain.']
  const failed = sum(['failed'])
  if (failed > 0) parts.push(`${plural(failed, 'job', 'jobs')} did not stop.`)
  if (left > 0) parts.push(`${plural(left, 'Claude session', 'Claude sessions')} kept running.`)
  if (recap.launchError) parts.push('The game did not start.')
  else if (recap.launchedInJob === true) parts.push('The game may close with Rain.')
  return parts.join(' ')
}
