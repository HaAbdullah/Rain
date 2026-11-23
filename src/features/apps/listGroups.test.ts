import { describe, expect, it } from 'vitest'
import type { AppEntry, Pool } from '@/data/types'
import { buildGroups } from './listGroups'

const icon = { kind: 'lucide', value: 'box' } as const

function pool(id: string, name: string): Pool {
  return { id, name, icon, members: [], pinned: false }
}

function app(id: string, name: string): AppEntry {
  return { id, name, icon, kind: 'program', keywords: [], pinned: false, running: false }
}

describe('buildGroups', () => {
  it('counts no pools when there are none, though the "New pool" row is listed', () => {
    const [pools] = buildGroups([], [app('a', 'Alpha')], '')
    expect(pools?.count).toBe(0)
    expect(pools?.items.map((row) => row.kind)).toEqual(['new'])
  })

  it('counts the pools and nothing else', () => {
    const [pools, apps] = buildGroups([pool('p1', 'Work'), pool('p2', 'Play')], [app('a', 'Alpha')], '')
    expect(pools?.count).toBe(2)
    expect(pools?.items).toHaveLength(2)
    expect(apps?.count).toBe(1)
  })

  it('offers no "New pool" row before the first answer or while searching', () => {
    expect(buildGroups(undefined, [], '')[0]).toMatchObject({ count: 0, items: [] })
    expect(buildGroups([], [], 'wo')[0]).toMatchObject({ count: 0, items: [] })
  })

  it('counts only the pools that match the search', () => {
    const [pools] = buildGroups([pool('p1', 'Work'), pool('p2', 'Play')], [], 'wor')
    expect(pools?.count).toBe(1)
  })
})
