import { describe, expect, it } from 'vitest'
import type { Game, GameSourceStatus } from '@/data/types'
import { errorMessages, gamesOfSub, resolveSub, visibleSubs } from './library-view'

const ok = (source: GameSourceStatus['source']): GameSourceStatus => ({ source, state: 'ok' })

function game(id: string, over: Partial<Game> = {}): Game {
  return {
    id, title: id, source: 'steam', installed: true, running: false, playtimeMinutes: 0,
    pinned: false, hidden: false, ...over,
  }
}

describe('visibleSubs', () => {
  it('has no tab for a source that is off', () => {
    const sources: GameSourceStatus[] = [ok('steam'), { source: 'epic', state: 'off', message: 'Epic Games is not installed' }, ok('custom')]
    expect(visibleSubs(sources, 0)).toEqual(['all', 'steam', 'custom', 'installed'])
  })

  it('shows Hidden only while a game is hidden', () => {
    const sources = [ok('steam'), ok('epic'), ok('custom')]
    expect(visibleSubs(sources, 0)).not.toContain('hidden')
    expect(visibleSubs(sources, 2)).toEqual(['all', 'steam', 'epic', 'custom', 'installed', 'hidden'])
  })

  it('keeps every tab before the first answer', () => {
    expect(visibleSubs([], 0)).toEqual(['all', 'steam', 'epic', 'custom', 'installed'])
  })

  it('does not hide a source that failed: it shows its warning', () => {
    expect(visibleSubs([{ source: 'epic', state: 'failed', message: 'x' }], 0)).toContain('epic')
  })
})

describe('resolveSub', () => {
  it('falls back to All when the tab is gone', () => {
    const visible = visibleSubs([{ source: 'epic', state: 'off' }], 0)
    expect(resolveSub('epic', visible)).toBe('all')
    expect(resolveSub('hidden', visible)).toBe('all')
    expect(resolveSub('steam', visible)).toBe('steam')
    expect(resolveSub(undefined, visible)).toBe('all')
    expect(resolveSub('nonsense', visible)).toBe('all')
  })
})

describe('gamesOfSub', () => {
  const games = [
    game('a'),
    game('b', { source: 'custom', installed: false }),
    game('c', { hidden: true }),
    game('d', { source: 'epic', hidden: true }),
  ]

  it('leaves hidden games out of every tab but Hidden, and counts them there', () => {
    expect(gamesOfSub(games, 'all').map((g) => g.id)).toEqual(['a', 'b'])
    expect(gamesOfSub(games, 'steam').map((g) => g.id)).toEqual(['a'])
    expect(gamesOfSub(games, 'custom').map((g) => g.id)).toEqual(['b'])
    expect(gamesOfSub(games, 'installed').map((g) => g.id)).toEqual(['a'])
    expect(gamesOfSub(games, 'hidden').map((g) => g.id)).toEqual(['c', 'd'])
  })
})

describe('errorMessages', () => {
  const sources: GameSourceStatus[] = [
    ok('steam'),
    { source: 'epic', state: 'failed', message: 'Could not read the Epic Games library' },
    { source: 'custom', state: 'failed', message: 'Game folder not found: D:\\Games' },
  ]

  it('gives a failed source its message and a warning only on its own tabs', () => {
    expect(errorMessages(sources, 'all', false)).toEqual([
      'Could not read the Epic Games library',
      'Game folder not found: D:\\Games',
    ])
    expect(errorMessages(sources, 'custom', false)).toEqual(['Game folder not found: D:\\Games'])
    expect(errorMessages(sources, 'steam', false)).toEqual([])
    expect(errorMessages(sources, 'hidden', false)).toEqual([])
  })

  it('gives one message when the whole library failed', () => {
    expect(errorMessages(sources, 'all', true)).toEqual(['Games unavailable'])
  })

  it('names a failed source that has no message', () => {
    expect(errorMessages([{ source: 'custom', state: 'failed' }], 'all', false)).toEqual(['Local library unavailable'])
  })

  it('says nothing about a source that is off', () => {
    expect(errorMessages([{ source: 'epic', state: 'off', message: 'x' }], 'all', false)).toEqual([])
  })
})
