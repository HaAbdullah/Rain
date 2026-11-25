import { describe, expect, it } from 'vitest'
import type { GameModeRecap, RecapItem } from '@/data/types'
import { recapText } from './recap'

const item = (over: Partial<RecapItem>): RecapItem => ({
  kind: 'job',
  label: 'Scheduled jobs',
  count: 1,
  outcome: 'stopped',
  names: [],
  ...over,
})

const recap = (over: Partial<GameModeRecap> = {}): GameModeRecap => ({
  at: '2026-11-22T10:00:00.000Z',
  complete: true,
  items: [],
  seen: false,
  ...over,
})

describe('recapText', () => {
  it('says Rain closed when nothing was stopped', () => {
    expect(recapText(recap())).toBe('Game mode closed Rain.')
  })

  it('counts stopped and forced jobs together, and sessions that kept running', () => {
    const text = recapText(
      recap({
        items: [
          item({ kind: 'session', label: 'Claude sessions', count: 3, outcome: 'leftRunning' }),
          item({ count: 1 }),
          item({ kind: 'idleJob', count: 1, outcome: 'forced' }),
        ],
      }),
    )
    expect(text).toBe('Game mode stopped 2 jobs. 3 Claude sessions kept running.')
  })

  it('uses the singular', () => {
    const text = recapText(
      recap({ items: [item({}), item({ kind: 'session', count: 1, outcome: 'leftRunning' })] }),
    )
    expect(text).toBe('Game mode stopped 1 job. 1 Claude session kept running.')
  })

  it('says when a stop failed', () => {
    const text = recapText(recap({ items: [item({ count: 1 }), item({ outcome: 'failed', count: 2 })] }))
    expect(text).toBe('Game mode stopped 1 job. 2 jobs did not stop.')
  })

  it('counts the other programs that were closed, with the jobs', () => {
    const text = recapText(
      recap({
        items: [
          item({ kind: 'otherApp', label: 'Other apps', count: 5 }),
          item({ kind: 'otherApp', label: 'Other apps', count: 2, outcome: 'forced' }),
          item({ count: 1 }),
        ],
      }),
    )
    expect(text).toBe('Game mode closed 7 apps and stopped 1 job.')
  })

  it('says which programs did not close, apart from the Claude sessions that kept running', () => {
    const text = recapText(
      recap({
        items: [
          item({ kind: 'otherApp', count: 1 }),
          item({ kind: 'otherApp', count: 1, outcome: 'leftRunning', names: ['Word'] }),
          item({ kind: 'otherApp', count: 2, outcome: 'failed' }),
          item({ kind: 'session', count: 2, outcome: 'leftRunning' }),
        ],
      }),
    )
    expect(text).toBe(
      'Game mode closed 1 app. 1 app did not close. 2 apps need administrator rights. 2 Claude sessions kept running.',
    )
  })

  it('says when the game did not start', () => {
    expect(recapText(recap({ launchError: 'It failed.' }))).toBe('Game mode closed Rain. The game did not start.')
  })

  it('says when the wind-down never finished, and nothing else', () => {
    expect(recapText(recap({ complete: false, items: [item({})] }))).toBe('Game mode did not finish.')
  })

  it('warns when the backend says the game may close with Rain', () => {
    const flagged = { ...recap(), launchedInJob: true }
    expect(recapText(flagged)).toBe('Game mode closed Rain. The game may close with Rain.')
  })

  it('says nothing about it without the flag, or when the game did not start', () => {
    expect(recapText(recap())).not.toContain('may close')
    const failed = { ...recap({ launchError: 'No program' }), launchedInJob: true }
    expect(recapText(failed)).not.toContain('may close')
  })
})
