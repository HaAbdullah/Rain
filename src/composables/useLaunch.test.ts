// The launch composable: a lost final frame must not leave the window held open for ever
// (section 01 9.3). The api and the shell are faked; time is faked too.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import type { LaunchRun } from '@/data/types'

const fakes = vi.hoisted(() => {
  const state = {
    frames: [] as ((run: unknown) => void)[],
    resets: [] as (() => void)[],
  }
  return {
    state,
    release: vi.fn(),
    hide: vi.fn(() => Promise.resolve()),
    api: {
      launch: vi.fn(),
      getLaunch: vi.fn(),
      subscribe: vi.fn((_topic: string, callback: (run: unknown) => void) => {
        state.frames.push(callback)
        return () => {}
      }),
    },
  }
})

vi.mock('@/data', () => ({
  api: fakes.api,
  onStreamReset: (callback: () => void) => {
    fakes.state.resets.push(callback)
    return () => {}
  },
}))
vi.mock('@/shell', () => ({
  shell: { holdOpen: () => fakes.release, keepOpen: { value: false }, hide: fakes.hide },
}))
vi.mock('./useResource', () => ({ refreshResources: vi.fn() }))

const TARGET = { type: 'app', id: 'app-a' } as const

function run(status: LaunchRun['status'], id = 'launch-1'): LaunchRun {
  return {
    id,
    target: { ...TARGET },
    status,
    steps: [{ label: 'A', icon: { kind: 'letter', value: 'A' }, status: status === 'running' ? 'running' : status }],
  }
}

async function load() {
  vi.resetModules()
  return await import('./useLaunch')
}

beforeEach(() => {
  vi.useFakeTimers()
  vi.stubGlobal('window', globalThis)
  fakes.state.frames.length = 0
  fakes.state.resets.length = 0
  fakes.release.mockClear()
  fakes.hide.mockClear()
  fakes.api.launch.mockReset()
  fakes.api.getLaunch.mockReset()
})

afterEach(() => {
  vi.useRealTimers()
  vi.unstubAllGlobals()
})

describe('useLaunch', () => {
  it('resolves when the final frame arrives and releases the window', async () => {
    const { useLaunch } = await load()
    fakes.api.launch.mockResolvedValue(run('running'))

    const pending = useLaunch().launch({ ...TARGET }, 'A')
    await vi.advanceTimersByTimeAsync(0)
    fakes.state.frames[0](run('success'))

    expect((await pending).status).toBe('success')
    expect(fakes.release).toHaveBeenCalledTimes(1)
    expect(fakes.api.getLaunch).not.toHaveBeenCalled()
  })

  it('tells both callers when two launches of the same running target end', async () => {
    const { useLaunch } = await load()
    // The backend hands back the run that is still going for the second call.
    fakes.api.launch.mockResolvedValue(run('running'))

    const first = useLaunch().launch({ ...TARGET }, 'A')
    const second = useLaunch().launch({ ...TARGET }, 'A')
    await vi.advanceTimersByTimeAsync(0)
    fakes.state.frames[0](run('success'))

    expect((await first).status).toBe('success')
    expect((await second).status).toBe('success')
    expect(fakes.release).toHaveBeenCalledTimes(2)
  })

  it('repairs a lost final frame with the poll after five seconds', async () => {
    const { useLaunch } = await load()
    fakes.api.launch.mockResolvedValue(run('running'))
    fakes.api.getLaunch.mockResolvedValue(run('success'))

    const pending = useLaunch().launch({ ...TARGET }, 'A')
    await vi.advanceTimersByTimeAsync(4900)
    expect(fakes.api.getLaunch).not.toHaveBeenCalled()
    await vi.advanceTimersByTimeAsync(200)

    expect((await pending).status).toBe('success')
    expect(fakes.api.getLaunch).toHaveBeenCalledWith('launch-1')
    expect(fakes.release).toHaveBeenCalledTimes(1)
  })

  it('keeps asking every three seconds while the run is still going', async () => {
    const { useLaunch } = await load()
    fakes.api.launch.mockResolvedValue(run('running'))
    fakes.api.getLaunch.mockResolvedValueOnce(run('running')).mockResolvedValue(run('failed'))

    const pending = useLaunch().launch({ ...TARGET }, 'A')
    await vi.advanceTimersByTimeAsync(5100)
    expect(fakes.api.getLaunch).toHaveBeenCalledTimes(1)
    await vi.advanceTimersByTimeAsync(3100)

    expect((await pending).status).toBe('failed')
    expect(fakes.api.getLaunch).toHaveBeenCalledTimes(2)
  })

  it('reconciles at once when the stream was opened again', async () => {
    const { useLaunch } = await load()
    fakes.api.launch.mockResolvedValue(run('running'))
    fakes.api.getLaunch.mockResolvedValue(run('success'))

    const pending = useLaunch().launch({ ...TARGET }, 'A')
    await vi.advanceTimersByTimeAsync(0)
    fakes.state.resets[0]()

    expect((await pending).status).toBe('success')
  })

  it('ends the wait as failed when the backend does not know the run any more', async () => {
    const { useLaunch } = await load()
    fakes.api.launch.mockResolvedValue(run('running'))
    fakes.api.getLaunch.mockRejectedValue({ code: 'notFound', message: 'Launch not found', reason: 'unknownRun' })

    const pending = useLaunch().launch({ ...TARGET }, 'A')
    await vi.advanceTimersByTimeAsync(5100)

    expect((await pending).status).toBe('failed')
    expect(fakes.release).toHaveBeenCalledTimes(1)
  })

  it('does not poll when nothing waits', async () => {
    const { useLaunch } = await load()
    fakes.api.launch.mockResolvedValue(run('success'))

    await useLaunch().launch({ ...TARGET }, 'A')
    await vi.advanceTimersByTimeAsync(20_000)

    expect(fakes.api.getLaunch).not.toHaveBeenCalled()
    expect(vi.getTimerCount()).toBe(0)
  })

  it('a failure that is not "not found" does not end the wait', async () => {
    const { useLaunch, resetLaunches } = await load()
    fakes.api.launch.mockResolvedValue(run('running'))
    fakes.api.getLaunch.mockRejectedValue({ code: 'unavailable', message: 'Rain took too long' })

    const pending = useLaunch().launch({ ...TARGET }, 'A')
    await vi.advanceTimersByTimeAsync(11_000)
    expect(fakes.release).not.toHaveBeenCalled()

    resetLaunches()
    expect((await pending).status).toBe('failed')
    expect(vi.getTimerCount()).toBe(0)
  })
})
