// Mock slice: drives and disk scans. A scan takes 4 seconds and reports
// progress on the 'scan' topic every 200 ms. Drive D: always stops at 40% with
// "Access denied"; in the Failing scenario every scan does.

import type { RainApi } from '../../api'
import type { DiskNode, DiskScan, Drive } from '../../types'
import { clone, delay, emit, every, fail, findOrFail } from '../engine'
import { options } from '../options'
import { DENIED_DRIVE_ID, buildTree, fileCount } from '../seed/disk'
import { nextId, state } from '../store'

const SCAN_MS = 4000
const TICK_MS = 200
const TICKS = SCAN_MS / TICK_MS
const FAILS_AT = 0.4

/** Stops the ticker of the scan in progress. Only one scan runs at a time. */
let stopTicker = (): void => {}

type DiskSlice = Pick<RainApi, 'listDrives' | 'startScan' | 'getScan' | 'cancelScan' | 'revealPath'>

export const diskSlice: DiskSlice = {
  async listDrives() {
    await delay()
    return clone(state.drives)
  },

  async startScan(driveId) {
    await delay()
    const drive = findOrFail(state.drives, driveId, 'Drive')
    cancelRunningScans()

    const scan: DiskScan = { id: nextId('scan'), driveId, status: 'scanning', progress: 0, filesSeen: 0 }
    state.scans[scan.id] = scan
    run(scan, drive)
    return snapshot(scan)
  },

  async getScan(id) {
    await delay()
    return snapshot(findScan(id))
  },

  async cancelScan(id) {
    await delay()
    const scan = findScan(id)
    if (scan.status !== 'scanning') return
    stopTicker()
    scan.status = 'cancelled'
    emit('scan', snapshot(scan))
  },

  async revealPath() {
    await delay()
  },
}

function run(scan: DiskScan, drive: Drive): void {
  const fails = drive.id === DENIED_DRIVE_ID || options.scenario === 'failing'
  const totalFiles = fileCount(drive)
  let ticks = 0

  stopTicker = every(TICK_MS, () => {
    ticks += 1
    scan.progress = ticks / TICKS
    scan.filesSeen = Math.round(totalFiles * scan.progress)

    if (fails && scan.progress >= FAILS_AT) {
      scan.status = 'failed'
      scan.error = 'Access denied'
      stopTicker()
    } else if (ticks === TICKS) {
      scan.status = 'done'
      scan.root = treeOf(drive)
      stopTicker()
    }
    emit('scan', snapshot(scan))
  })
}

function cancelRunningScans(): void {
  stopTicker()
  for (const scan of Object.values(state.scans)) {
    if (scan.status !== 'scanning') continue
    scan.status = 'cancelled'
    emit('scan', snapshot(scan))
  }
}

function findScan(id: string): DiskScan {
  const scan = state.scans[id]
  if (!scan) throw fail('notFound', 'Scan not found')
  return scan
}

/** Built on the first finished scan of a drive, then reused. */
function treeOf(drive: Drive): DiskNode {
  state.trees[drive.id] ??= buildTree(drive, options.scenario)
  return state.trees[drive.id]
}

/**
 * A copy of the scan's own fields. The tree is handed out as it is, not copied:
 * it has thousands of nodes and nothing ever changes it.
 */
function snapshot(scan: DiskScan): DiskScan {
  return { ...scan }
}
