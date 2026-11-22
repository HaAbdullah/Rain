// Keeps the scanned tree for 5 minutes after the user leaves Disk (spec 7.7).
// The tree itself lives in `diskView` (actions.ts); this file only decides when to drop it.
import { onScopeDispose } from 'vue'
import { resetDisk } from './actions'

const KEEP_MS = 5 * 60 * 1000

/** The one timer: it runs only while Disk is not on screen. */
let dropTimer: number | undefined

/** Call once in the Disk screen's setup: coming back cancels the drop, leaving starts it. */
export function keepTreeWhileAway(): void {
  window.clearTimeout(dropTimer)
  onScopeDispose(() => {
    dropTimer = window.setTimeout(resetDisk, KEEP_MS)
  })
}
