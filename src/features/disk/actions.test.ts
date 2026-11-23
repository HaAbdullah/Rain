import { afterEach, describe, expect, it, vi } from 'vitest'
import type { ActionContext } from '@/actions/types'
import type { DiskNode, DiskScan } from '@/data/types'

// actions.ts imports the data layer, which would start a shell and a stream.
const backend = vi.hoisted(() => ({ cancelScan: vi.fn() }))
vi.mock('@/data', () => ({ api: backend }))

import { diskNodeActions, diskView, reconcileDisk, resetDisk, scanDrive } from './actions'

const scanning = (over: Partial<DiskScan> = {}): DiskScan => ({
  id: 'scan-1',
  driveId: 'drive-c',
  status: 'scanning',
  progress: 0.1,
  filesSeen: 10,
  ...over,
})
const root: DiskNode = {
  name: 'C:',
  path: 'C:\\',
  sizeBytes: 100,
  kind: 'dir',
  children: [{ name: 'big', path: 'C:\\big', sizeBytes: 90, kind: 'dir', truncated: true, category: 'video' }],
}
const done = (over: Partial<DiskScan> = {}): DiskScan => ({ ...scanning(), status: 'done', progress: 1, ...over })

/** A fake data layer that records its calls and lets the test push frames. */
function fakeContext(answers: { start: DiskScan; get: Array<DiskScan | Error>; node?: DiskNode }) {
  let push: (scan: DiskScan) => void = () => {}
  const released: Array<DiskScan | null> = []
  const get = [...answers.get]
  const api = {
    startScan: vi.fn(async () => answers.start),
    getScan: vi.fn(async () => {
      const next = get.shift() ?? done({ root })
      if (next instanceof Error) throw next
      return next
    }),
    getScanNode: vi.fn(async () => answers.node as DiskNode),
    subscribe: vi.fn((_topic: string, cb: (scan: DiskScan) => void) => {
      push = cb
      return () => {}
    }),
  }
  const shell = {
    // What the scan shows at the moment the hold on the window ends.
    holdOpen: vi.fn(() => () => released.push(diskView.scan.value)),
  }
  const context = { api, shell } as unknown as ActionContext
  return { api, context, released, push: (scan: DiskScan) => push(scan) }
}

/** Lets the promises of the actions run to their end. */
const settle = () => new Promise((resolve) => setTimeout(resolve, 0))

afterEach(() => {
  backend.cancelScan.mockReset()
  diskView.scan.value = null
  diskView.folderPath.value = null
  diskView.loaded.clear()
})

describe('scanDrive', () => {
  it('fetches the tree with one getScan when a done frame arrives, and releases the window after it is stored', async () => {
    const fake = fakeContext({ start: scanning(), get: [scanning()] })
    await scanDrive('drive-c', fake.context)
    await settle()
    expect(fake.api.getScan).toHaveBeenCalledTimes(1) // the check right after subscribing

    fake.push(done())
    await settle()
    expect(fake.api.getScan).toHaveBeenCalledTimes(2) // one more for the tree
    expect(diskView.scan.value?.root).toEqual(root)
    expect(fake.released).toHaveLength(1)
    expect(fake.released[0]?.root).toEqual(root) // the hold ended after the result was stored
  })

  it('repairs a scan that ended before the subscription existed', async () => {
    const fake = fakeContext({ start: scanning(), get: [done({ root })] })
    await scanDrive('drive-c', fake.context)
    await settle()
    expect(diskView.scan.value?.status).toBe('done')
    expect(diskView.scan.value?.root).toEqual(root)
    expect(fake.released).toHaveLength(1)
  })

  it('ends as failed with a message when the tree cannot be fetched, and still releases the window', async () => {
    const fake = fakeContext({ start: scanning(), get: [scanning(), new Error('boom')] })
    await scanDrive('drive-c', fake.context)
    await settle()
    fake.push(done())
    await settle()
    expect(diskView.scan.value?.status).toBe('failed')
    expect(diskView.scan.value?.error).toBeTruthy()
    expect(fake.released).toHaveLength(1)
  })

  it('shows a failed scan as it is and releases the window', async () => {
    const fake = fakeContext({ start: scanning(), get: [scanning()] })
    await scanDrive('drive-c', fake.context)
    await settle()
    fake.push(scanning({ status: 'failed', error: 'Drive C: could not be read' }))
    await settle()
    expect(diskView.scan.value).toMatchObject({ status: 'failed', error: 'Drive C: could not be read' })
    expect(fake.released).toHaveLength(1)
  })

  it('keeps the newest count when a late frame arrives out of order', async () => {
    const fake = fakeContext({ start: scanning(), get: [scanning({ filesSeen: 500 })] })
    await scanDrive('drive-c', fake.context)
    await settle()
    fake.push(scanning({ filesSeen: 20 }))
    expect(diskView.scan.value?.filesSeen).toBe(500)
  })

  it('reconcileDisk asks how the followed scan stands', async () => {
    const fake = fakeContext({ start: scanning(), get: [scanning(), done({ root })] })
    await scanDrive('drive-c', fake.context)
    await settle()
    await reconcileDisk()
    expect(diskView.scan.value?.status).toBe('done')
    expect(fake.released).toHaveLength(1)
  })
})

describe('resetDisk', () => {
  it('frees the tree in the backend once and ignores a rejection', async () => {
    backend.cancelScan.mockRejectedValue(new Error('unknown scan'))
    diskView.scan.value = done({ root })
    expect(() => resetDisk()).not.toThrow()
    await settle()
    expect(backend.cancelScan).toHaveBeenCalledTimes(1)
    expect(backend.cancelScan).toHaveBeenCalledWith('scan-1')
    expect(diskView.scan.value).toBeNull()
    expect(diskView.folderPath.value).toBeNull()
  })

  it('does not call the backend when there is no scan', () => {
    resetDisk()
    expect(backend.cancelScan).not.toHaveBeenCalled()
  })
})

describe('Disk node actions', () => {
  function actionsFor(node: DiskNode) {
    return diskNodeActions({ type: 'diskNode', id: node.path, item: node }, undefined as never)
  }

  it('fetches a truncated folder once before opening it, and not again', async () => {
    const big = root.children![0]
    const whole: DiskNode = {
      ...big,
      truncated: undefined,
      category: undefined,
      children: [{ name: 'f', path: 'C:\\big\\f', sizeBytes: 90, kind: 'file', category: 'video' }],
    }
    const fake = fakeContext({ start: scanning(), get: [], node: whole })
    diskView.scan.value = done({ root })

    const open = () => actionsFor(big).find((action) => action.id === 'open-folder')!
    await open().run(fake.context)
    expect(fake.api.getScanNode).toHaveBeenCalledTimes(1)
    expect(fake.api.getScanNode).toHaveBeenCalledWith('scan-1', 'C:\\big')
    expect(diskView.folderPath.value).toBe('C:\\big')
    expect(diskView.scan.value?.root?.children?.[0]).toEqual(whole)

    await open().run(fake.context)
    expect(fake.api.getScanNode).toHaveBeenCalledTimes(1)
  })

  it('does not fetch a complete folder or a denied one', async () => {
    const fake = fakeContext({ start: scanning(), get: [] })
    diskView.scan.value = done({ root })
    const complete: DiskNode = { name: 'c', path: 'C:\\c', sizeBytes: 1, kind: 'dir' }
    const denied: DiskNode = { name: 'd', path: 'C:\\d', sizeBytes: 0, kind: 'dir', denied: true }
    for (const node of [complete, denied]) {
      await actionsFor(node).find((action) => action.id === 'open-folder')!.run(fake.context)
      expect(diskView.folderPath.value).toBe(node.path)
    }
    expect(fake.api.getScanNode).not.toHaveBeenCalled()
  })

  it('offers nothing for an other row', () => {
    const other: DiskNode = { name: 'other (3 items)', path: 'C:\\*', sizeBytes: 3, kind: 'other' }
    expect(actionsFor(other)).toEqual([])
  })
})
