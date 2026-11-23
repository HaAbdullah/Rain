import { describe, expect, it } from 'vitest'
import type { Project } from '@/data/types'
import { byLastOpened, openedAt } from './project-order'

function project(name: string, lastOpenedAt?: string): Project {
  return {
    id: `proj-${name}`,
    name,
    path: `D:\\Work\\${name}`,
    icon: { kind: 'letter', value: name },
    branch: 'main',
    commits14d: new Array<number>(14).fill(0),
    lastOpenedAt,
    activityScore: 0,
    environment: [],
    open: false,
    pinned: false,
    missing: false,
  }
}

describe('byLastOpened', () => {
  it('puts the newest open first and never-opened projects last, in the order they came', () => {
    const list = [
      project('never-a'),
      project('old', '2026-11-01T00:00:00.000Z'),
      project('never-b'),
      project('new', '2026-11-20T00:00:00.000Z'),
    ]
    expect([...list].sort(byLastOpened).map((p) => p.name)).toEqual(['new', 'old', 'never-a', 'never-b'])
  })

  it('reads a missing or unreadable time as 0', () => {
    expect(openedAt(project('x'))).toBe(0)
    expect(openedAt(project('x', 'not a date'))).toBe(0)
    expect(openedAt(project('x', '1970-01-02T00:00:00.000Z'))).toBe(86_400_000)
  })
})
