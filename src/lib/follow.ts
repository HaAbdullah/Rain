// Follows a log that is still being written (a run in progress): load it, ask whether it is
// still live, and when it is, load it again a moment later. A pure function with injected
// timers, so a test can drive it without waiting. Section 03 9.1.
//
// The loop exists only while its owner keeps it started (the log dialog is open and the window
// is visible), so it follows rule 14 of the UI spec: no intervals while the window is hidden.

export interface Follower {
  /** Loads at once, then keeps loading while the log is live. Starting again restarts the loop. */
  start(): void
  /** Cancels the pending reload. A load that is already under way is ignored when it ends. */
  stop(): void
}

export interface FollowerOptions {
  /** Reloads the lines without clearing them. A rejection ends the loop. */
  load: () => Promise<void>
  /** True while the run is still going. A rejection ends the loop. */
  isLive: () => Promise<boolean>
  /** The pause between two loads, in milliseconds (3000 in the app). */
  everyMs: number
  setTimer?: typeof setTimeout
  clearTimer?: typeof clearTimeout
}

export function createFollower(options: FollowerOptions): Follower {
  const { load, isLive, everyMs, setTimer = setTimeout, clearTimer = clearTimeout } = options
  let timer: ReturnType<typeof setTimeout> | undefined
  // Every start and stop makes the work of the earlier loop obsolete.
  let generation = 0

  async function step(mine: number, wasLive: boolean): Promise<void> {
    try {
      await load()
      if (mine !== generation) return
      const live = await isLive()
      if (mine !== generation) return
      if (!live) {
        // The run ended after the load above started: one more load brings its closing lines.
        if (wasLive) await load()
        return
      }
    } catch {
      return
    }
    timer = setTimer(() => {
      timer = undefined
      void step(mine, true)
    }, everyMs)
  }

  function stop(): void {
    generation += 1
    if (timer !== undefined) {
      clearTimer(timer)
      timer = undefined
    }
  }

  function start(): void {
    stop()
    void step(generation, false)
  }

  return { start, stop }
}
