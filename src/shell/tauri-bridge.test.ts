import { beforeEach, describe, expect, it, vi } from 'vitest'
import type { StreamFrame } from './bridge'
import { createStream } from '@/data/rest/stream'

// The Tauri API only exists inside the window, so it is replaced by two small fakes.
const invoke = vi.fn()
vi.mock('@tauri-apps/api/core', () => ({ invoke: (...args: unknown[]) => invoke(...args) }))
vi.mock('@tauri-apps/api/event', () => ({ listen: () => Promise.resolve(() => {}) }))

import { createTauriBridge } from './tauri-bridge'

/** Lets the promises of `openStream` (the listeners, then the state question) run. */
const settle = () => new Promise((resolve) => setTimeout(resolve, 0))

beforeEach(() => invoke.mockReset())

describe('tauri bridge: a page that starts after the backend is up', () => {
  it('turns the answer of /local/backend into a hello with the start id and a resync', async () => {
    invoke.mockResolvedValue({ state: 'up', startId: 'start-1' })
    const frames: StreamFrame[] = []

    createTauriBridge().openStream({ onFrame: (frame) => frames.push(frame), onState: () => {} })
    await settle()

    expect(frames.map((frame) => frame.topic)).toEqual(['hello', 'resync'])
    expect(frames[0].payload).toEqual({ startId: 'start-1' })
  })

  it('so that a later restart of the backend is seen as a restart', async () => {
    invoke.mockResolvedValue({ state: 'up', startId: 'start-1' })
    let push: (frame: StreamFrame) => void = () => {}
    const bridge = createTauriBridge()
    const open = bridge.openStream.bind(bridge)
    const stream = createStream({
      ...bridge,
      openStream: (handlers) => {
        push = handlers.onFrame
        return open(handlers)
      },
    })
    const resets: boolean[] = []
    stream.onReset(({ restarted }) => resets.push(restarted))
    await settle()

    push({ topic: 'hello', seq: 0, payload: { startId: 'start-2' } })
    push({ topic: 'resync', seq: 1, payload: { reason: 'connect' } })

    expect(resets).toEqual([false, true])
  })

  it('does not invent frames when the backend is not up', async () => {
    invoke.mockResolvedValue({ state: 'down' })
    const frames: StreamFrame[] = []
    const states: string[] = []

    createTauriBridge().openStream({ onFrame: (frame) => frames.push(frame), onState: (state) => states.push(state) })
    await settle()

    expect(frames).toEqual([])
    expect(states).toEqual(['down'])
  })
})

describe('tauri bridge: media', () => {
  it('builds media addresses on the rain-media custom scheme (Windows form, http)', () => {
    expect(createTauriBridge().mediaUrl('media/app-icon/app-vscode?v=3fa9')).toBe(
      'http://rain-media.localhost/media/app-icon/app-vscode?v=3fa9',
    )
  })
})
