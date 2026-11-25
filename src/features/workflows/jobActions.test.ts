import { describe, expect, it, vi } from 'vitest'
import type { ActionContext } from '@/actions/types'
import type { ScheduledJob } from '@/data/types'
import { jobActions } from './actions'

// The actions never call the api here; the real one needs a browser.
vi.mock('@/data', () => ({ api: {} }))

function job(source: ScheduledJob['source']): ScheduledJob {
  return {
    id: source === 'rain' ? 'job-a' : 'claude-app:a',
    source,
    name: 'A',
    kind: 'claude',
    cron: '0 6 * * *',
    scheduleLabel: 'Every day at 06:00',
    enabled: true,
    running: false,
    recentRuns: [],
  }
}

function ids(item: ScheduledJob, where: ActionContext['where']): string[] {
  const target = { type: 'job', id: item.id, item } as const
  return jobActions(target, { where } as ActionContext).map((action) => action.id)
}

describe('jobActions', () => {
  it('gives a job of Rain the switch, the log and Run now', () => {
    expect(ids(job('rain'), 'section')).toEqual(['log', 'toggle', 'run-now', 'copy'])
  })

  it('gives a job of the Claude app no switch, no log and no Run now', () => {
    expect(ids(job('claude-app'), 'section')).toEqual(['copy'])
    expect(ids(job('claude-app'), 'home')).toEqual(['show'])
  })
})
