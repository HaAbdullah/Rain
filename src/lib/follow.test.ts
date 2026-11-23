import { describe, expect, it, vi } from 'vitest'
import { createFollower } from './follow'

/** Lets the promises of the follower settle. */
const flush = () => new Promise<void>((resolve) => setTimeout(resolve, 0))

/** A timer that only fires when the test says so. */
function manualTimers() {
  const pending = new Map<number, () => void>()
  let next = 1
  return {
    setTimer: ((callback: () => void) => {
      const id = next++
      pending.set(id, callback)
      return id
    }) as unknown as typeof setTimeout,
    clearTimer: ((id: number) => {
      pending.delete(id)
    }) as unknown as typeof clearTimeout,
    count: () => pending.size,
    /** Fires the timers that wait now. */
    async fire() {
      const due = [...pending.values()]
      pending.clear()
      due.forEach((callback) => callback())
      await flush()
    },
  }
}

function setup(live: () => boolean | Promise<boolean>, load: () => Promise<void> = () => Promise.resolve()) {
  const timers = manualTimers()
  const loads = vi.fn(load)
  const lives = vi.fn(async () => live())
  const follower = createFollower({ load: loads, isLive: lives, everyMs: 3000, ...timers })
  return { follower, timers, loads, lives }
}

describe('createFollower', () => {
  it('loads at once and reloads while the log is live', async () => {
    const { follower, timers, loads } = setup(() => true)
    follower.start()
    await flush()
    expect(loads).toHaveBeenCalledTimes(1)
    expect(timers.count()).toBe(1)

    await timers.fire()
    await timers.fire()
    expect(loads).toHaveBeenCalledTimes(3)
    follower.stop()
  })

  it('loads once more when the run ended after the last load, then stops', async () => {
    let live = true
    const { follower, timers, loads } = setup(() => live)
    follower.start()
    await flush()
    live = false
    await timers.fire()
    // The load of the tick, and the one that brings the closing lines of the run.
    expect(loads).toHaveBeenCalledTimes(3)
    expect(timers.count()).toBe(0)
  })

  it('does not wait for another load when the run is already over', async () => {
    const { follower, timers, loads } = setup(() => false)
    follower.start()
    await flush()
    expect(loads).toHaveBeenCalledTimes(1)
    expect(timers.count()).toBe(0)
  })

  it('cancels the pending reload on stop', async () => {
    const { follower, timers, loads } = setup(() => true)
    follower.start()
    await flush()
    follower.stop()
    expect(timers.count()).toBe(0)
    await timers.fire()
    expect(loads).toHaveBeenCalledTimes(1)
  })

  it('ignores a load that ends after stop', async () => {
    let finish: () => void = () => {}
    const { follower, timers, lives } = setup(
      () => true,
      () => new Promise<void>((resolve) => (finish = resolve)),
    )
    follower.start()
    follower.stop()
    finish()
    await flush()
    expect(lives).not.toHaveBeenCalled()
    expect(timers.count()).toBe(0)
  })

  it('stops when a load fails', async () => {
    const { follower, timers } = setup(
      () => true,
      () => Promise.reject(new Error('no backend')),
    )
    follower.start()
    await flush()
    expect(timers.count()).toBe(0)
  })

  it('stops when the question whether the run is live fails', async () => {
    const { follower, timers } = setup(() => Promise.reject(new Error('no backend')))
    follower.start()
    await flush()
    expect(timers.count()).toBe(0)
  })

  it('starting again restarts the loop instead of running two', async () => {
    const { follower, timers, loads } = setup(() => true)
    follower.start()
    await flush()
    follower.start()
    await flush()
    expect(loads).toHaveBeenCalledTimes(2)
    expect(timers.count()).toBe(1)
    follower.stop()
  })
})
