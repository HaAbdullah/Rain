// Actions for drives and scanned files and folders (spec 4.3), plus the small
// state the Disk screen shares with them. This file must stay free of
// d3-hierarchy: the registry imports it at start-up, while the Disk screen is a lazy chunk.
import { ref, shallowRef } from 'vue'
import { copyAction, revealAction } from '@/actions/common'
import type { ActionContext, ActionProvider, RainAction, TargetOf } from '@/actions/types'
import { api } from '@/data'
import type { RainApi } from '@/data/api'
import type { DiskNode, DiskScan, Drive } from '@/data/types'
import { errorMessage } from '@/lib/errors'
import { needsLoad, replaceNode } from './folders'
import { newer } from './scanState'

/**
 * What the Disk screen shows. The actions write it; DiskScreen reads it
 * (and may write `folderPath` for breadcrumb clicks and Backspace).
 */
export const diskView = {
  /** The newest scan, kept current from the 'scan' topic while it runs. null = no scan yet. */
  scan: shallowRef<DiskScan | null>(null),
  /** The folder shown in the result view. null = the drive's root. */
  folderPath: ref<string | null>(null),
  /** Folders of this scan that were fetched whole with getScanNode (so they are not fetched again). Not reactive on purpose. */
  loaded: new Set<string>(),
}

/** Ends the follow-up of the running scan: the subscription and the hold on the window. */
let stopFollowing: (() => void) | null = null

/** True while startScan is in flight, so a second Ctrl+R cannot start a second follow-up. */
let starting = false

/** Forgets the scan and the open folder, and tells the backend to free the tree. For a restart and the 5 minute drop. */
export function resetDisk(): void {
  stopFollowing?.()
  const scan = diskView.scan.value
  // Frees a finished tree; a running scan is cancelled; an id the backend no longer knows is ignored.
  if (scan) void api.cancelScan(scan.id).catch(() => {})
  diskView.scan.value = null
  diskView.folderPath.value = null
  diskView.loaded.clear()
}

/** Asks the backend how the followed scan stands (set while a scan is followed). */
let refresh: (() => Promise<void>) | null = null

/**
 * Starts a scan and follows it to its end. The window is held open meanwhile
 * (spec 2.4). A scan that cannot start rejects; one that fails later ends in
 * `diskView.scan` with status 'failed', which the screen shows as an ErrorRow.
 *
 * Three things can tell how the scan stands: a frame of the 'scan' topic, the answer of
 * getScan and a reconnect. The topic has no replay, so a scan that ends before the
 * subscription exists is found by asking once right after subscribing. `newer` decides
 * which snapshot wins; a final state beats 'scanning'. Frames never carry the tree: a done
 * scan is fetched with getScan.
 */
export async function scanDrive(driveId: Drive['id'], ctx: ActionContext): Promise<void> {
  if (starting) return
  starting = true
  let started: DiskScan
  try {
    started = await ctx.api.startScan(driveId)
  } finally {
    starting = false
  }
  // Only now that the scan exists is the old result replaced.
  stopFollowing?.()
  diskView.folderPath.value = null
  diskView.loaded.clear()
  diskView.scan.value = started
  if (started.status !== 'scanning') return

  const release = ctx.shell.holdOpen('scan')
  let finishing = false
  function end() {
    unsubscribe()
    release()
    stopFollowing = null
    refresh = null
  }
  /** One place decides what a snapshot means. */
  async function apply(incoming: DiskScan): Promise<void> {
    if (finishing || incoming.id !== started.id) return
    const merged = newer(diskView.scan.value, incoming)
    if (merged.status === 'scanning') {
      diskView.scan.value = merged
      return
    }
    finishing = true
    try {
      diskView.scan.value = merged.status === 'done' ? await withRoot(merged, ctx) : merged
    } catch (reason) {
      diskView.scan.value = { ...merged, status: 'failed', error: errorMessage(reason, 'Could not load the result') }
    } finally {
      end()
    }
  }
  const unsubscribe = ctx.api.subscribe('scan', (frame) => void apply(frame))
  stopFollowing = end
  refresh = async () => apply(await ctx.api.getScan(started.id))
  void refresh().catch(() => {}) // the scan may have ended before the subscription existed
}

/** A done frame carries no tree: fetch it. The hold on the window is released only after this. */
async function withRoot(done: DiskScan, ctx: ActionContext): Promise<DiskScan> {
  const full = done.root ? done : await ctx.api.getScan(done.id)
  if (!full.root) return { ...full, status: 'failed', error: 'The result is no longer available' }
  diskView.loaded.add(full.root.path)
  return full
}

/** After a stream reconnect that was not a restart (main.ts): ask how the followed scan stands. */
export async function reconcileDisk(): Promise<void> {
  await refresh?.().catch(() => {})
}

/** Starts a scan. Ctrl+R scans as well, so "rescan" has one key everywhere in Disk. */
export function scanAction(driveId: Drive['id']): RainAction {
  return {
    id: 'scan',
    label: 'Scan',
    icon: 'scan',
    slot: 'primary',
    keys: 'Ctrl+KeyR',
    run: (ctx) => scanDrive(driveId, ctx),
  }
}

/** Ctrl+Enter while a scan runs. Cancelling is explicit: Enter and Esc never do it. */
export function cancelScanAction(scan: DiskScan): RainAction {
  return {
    id: 'cancel-scan',
    label: 'Cancel scan',
    icon: 'x',
    slot: 'secondary',
    async run({ api, notice }) {
      await api.cancelScan(scan.id)
      notice.info('Scan cancelled')
    },
  }
}

function rescan(): RainAction | null {
  const scan = diskView.scan.value
  return scan ? { ...scanAction(scan.driveId), label: 'Rescan', icon: 'refresh-cw', slot: 'more' } : null
}

export const driveActions: ActionProvider<TargetOf<'drive'>> = ({ item: drive }) => {
  const scan = diskView.scan.value
  if (scan?.status === 'scanning' && scan.driveId === drive.id) {
    return [
      {
        id: 'scanning',
        label: 'Scanning',
        icon: 'loader-circle',
        slot: 'primary',
        // Enter has nothing to do while the scan runs; it must not restart or stop it.
        run() {},
      },
      cancelScanAction(scan),
    ]
  }
  return [scanAction(drive.id)]
}

/** Fetches the folder whole when its children are missing or partial, and puts it into the scan's tree. */
export async function loadFolder(node: DiskNode, backend: RainApi): Promise<void> {
  const scan = diskView.scan.value
  if (!scan?.root || !needsLoad(node, diskView.loaded)) return
  const full = await backend.getScanNode(scan.id, node.path)
  const now = diskView.scan.value
  if (now?.id !== scan.id || !now.root) return // a newer scan replaced this one while we waited
  diskView.loaded.add(node.path)
  diskView.scan.value = { ...now, root: replaceNode(now.root, full) }
}

export const diskNodeActions: ActionProvider<TargetOf<'diskNode'>> = ({ item: node }) => {
  const copyPath = copyAction('path', node.path)
  const rescanAction = rescan()
  const more = rescanAction ? [rescanAction] : []

  // An "other (1,204 items)" row stands for entries that are not listed one by one: nothing to open, reveal or copy.
  if (node.kind === 'other') return []

  // A file has nothing to open inside Rain, so Enter copies its path and
  // Ctrl+Enter reveals it, as for a folder.
  if (node.kind === 'file') {
    return [{ ...copyPath, slot: 'primary' }, revealAction(node.path, 'secondary'), ...more]
  }

  return [
    {
      id: 'open-folder',
      label: 'Open folder',
      icon: 'folder-open',
      slot: 'primary',
      async run({ api: backend }) {
        // Before navigating, so the list is whole when it appears (the screen selects the first row on a change).
        await loadFolder(node, backend)
        diskView.folderPath.value = node.path
      },
    },
    revealAction(node.path, 'secondary'),
    ...more,
    copyPath,
  ]
}
