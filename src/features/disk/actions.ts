// Actions for drives and scanned files and folders (spec 4.3), plus the small
// state the Disk screen shares with them. This file must stay free of
// d3-hierarchy: the registry imports it at start-up, while the Disk screen is a lazy chunk.
import { ref, shallowRef } from 'vue'
import { copyAction, revealAction } from '@/actions/common'
import type { ActionContext, ActionProvider, RainAction, TargetOf } from '@/actions/types'
import type { DiskScan, Drive } from '@/data/types'

/**
 * What the Disk screen shows. The actions write it; DiskScreen reads it
 * (and may write `folderPath` for breadcrumb clicks and Backspace).
 */
export const diskView = {
  /** The newest scan, kept current from the 'scan' topic while it runs. null = no scan yet. */
  scan: shallowRef<DiskScan | null>(null),
  /** The folder shown in the result view. null = the drive's root. */
  folderPath: ref<string | null>(null),
}

/** Ends the follow-up of the running scan: the subscription and the hold on the window. */
let stopFollowing: (() => void) | null = null

/** True while startScan is in flight, so a second Ctrl+R cannot start a second follow-up. */
let starting = false

/** Forgets the scan and the open folder. For a data reseed, which ends scans without a final event. */
export function resetDisk(): void {
  stopFollowing?.()
  diskView.scan.value = null
  diskView.folderPath.value = null
}

/**
 * Starts a scan and follows it to its end. The window is held open meanwhile
 * (spec 2.4). A scan that cannot start rejects; one that fails later ends in
 * `diskView.scan` with status 'failed', which the screen shows as an ErrorRow.
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
  diskView.scan.value = started
  if (started.status !== 'scanning') return

  const release = ctx.shell.holdOpen('scan')
  const unsubscribe = ctx.api.subscribe('scan', (scan) => {
    if (scan.id !== started.id) return
    diskView.scan.value = scan
    if (scan.status !== 'scanning') stop()
  })
  function stop() {
    unsubscribe()
    release()
    stopFollowing = null
  }
  stopFollowing = stop
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

export const diskNodeActions: ActionProvider<TargetOf<'diskNode'>> = ({ item: node }) => {
  const copyPath = copyAction('path', node.path)
  const rescanAction = rescan()
  const more = rescanAction ? [rescanAction] : []

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
      run() {
        diskView.folderPath.value = node.path
      },
    },
    revealAction(node.path, 'secondary'),
    ...more,
    copyPath,
  ]
}
