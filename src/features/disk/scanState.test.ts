import { describe, expect, it } from 'vitest'
import type { DiskScan } from '@/data/types'
import { newer } from './scanState'

const scan = (over: Partial<DiskScan>): DiskScan => ({
  id: 'scan-1',
  driveId: 'drive-c',
  status: 'scanning',
  progress: 0,
  filesSeen: 0,
  ...over,
})

describe('newer', () => {
  it('takes the first snapshot it sees', () => {
    const incoming = scan({ filesSeen: 5 })
    expect(newer(null, incoming)).toBe(incoming)
  })

  it('never replaces a final state', () => {
    for (const status of ['done', 'failed', 'cancelled'] as const) {
      const current = scan({ status, filesSeen: 10 })
      expect(newer(current, scan({ status: 'scanning', filesSeen: 99 }))).toBe(current)
      expect(newer(current, scan({ status: 'failed' }))).toBe(current)
    }
  })

  it('keeps the larger count when two scanning snapshots arrive in reverse order', () => {
    const later = scan({ filesSeen: 200 })
    const earlier = scan({ filesSeen: 100 })
    expect(newer(later, earlier)).toBe(later)
    expect(newer(earlier, later)).toBe(later)
  })

  it('lets a snapshot of another scan replace the current one', () => {
    const current = scan({ id: 'scan-1', status: 'done' })
    const incoming = scan({ id: 'scan-2', filesSeen: 1 })
    expect(newer(current, incoming)).toBe(incoming)
  })

  it('lets done win over scanning', () => {
    const done = scan({ status: 'done', progress: 1, filesSeen: 300 })
    expect(newer(scan({ filesSeen: 250 }), done)).toBe(done)
    // Even when the final snapshot saw fewer files (it cannot, but the rule must not depend on it).
    expect(newer(scan({ filesSeen: 400 }), done)).toBe(done)
  })
})
