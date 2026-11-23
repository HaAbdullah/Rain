import { describe, expect, it, vi } from 'vitest'
import { createFakeBridge, FAKE_MEDIA_ORIGIN, type FakeBridge } from './fake-bridge'
import { createStream } from './stream'

function frame(topic: string, payload: unknown, seq = 1) {
  return { topic, seq, payload }
}

function hello(bridge: FakeBridge, startId: string) {
  bridge.push(frame('hello', { startId, version: '0.1.0', apiVersion: 1, seq: 0 }))
}

function resync(bridge: FakeBridge, reason: 'connect' | 'lagged' = 'connect') {
  bridge.push(frame('resync', { reason }))
}

/** A stream plus a list that records the `restarted` flag of every reset. */
function setup() {
  const bridge = createFakeBridge()
  const stream = createStream(bridge)
  const resets: boolean[] = []
  stream.onReset(({ restarted }) => resets.push(restarted))
  return { bridge, stream, resets }
}

describe('stream client: resets', () => {
  it('opens the bridge stream when it is created, before any subscribe', () => {
    const bridge = createFakeBridge()
    createStream(bridge)
    expect(bridge.streamsOpened).toBe(1)
  })

  it('calls the reset callbacks on resync, and the first hello is not a restart', () => {
    const { bridge, resets } = setup()

    hello(bridge, 'start-1')
    resync(bridge)

    expect(resets).toEqual([false])
  })

  it('reports restarted when the next hello has a different startId, once', () => {
    const { bridge, resets } = setup()

    hello(bridge, 'start-1')
    resync(bridge)
    hello(bridge, 'start-2')
    resync(bridge)
    resync(bridge, 'lagged')

    expect(resets).toEqual([false, true, false])
  })

  it('reports no restart when the same backend process reconnects', () => {
    const { bridge, resets } = setup()

    hello(bridge, 'start-1')
    resync(bridge)
    hello(bridge, 'start-1')
    resync(bridge)

    expect(resets).toEqual([false, false])
  })

  it('a resync without a hello (a lag, or a page that started late) is a reset with restarted false', () => {
    const { bridge, resets } = setup()

    resync(bridge, 'lagged')

    expect(resets).toEqual([false])
  })

  it('an unsubscribed reset callback is not called again', () => {
    const bridge = createFakeBridge()
    const stream = createStream(bridge)
    let calls = 0
    const stop = stream.onReset(() => (calls += 1))

    resync(bridge)
    stop()
    resync(bridge)

    expect(calls).toBe(1)
  })
})

describe('stream client: topics', () => {
  it('delivers a topic frame to every subscriber of that topic, in order, and to no other', () => {
    const bridge = createFakeBridge()
    const stream = createStream(bridge)
    const first: string[] = []
    const second: string[] = []
    const other: unknown[] = []
    stream.subscribe('health', (payload) => first.push(payload.startId))
    stream.subscribe('health', (payload) => second.push(payload.startId))
    stream.subscribe('jobs', (payload) => other.push(payload))

    bridge.push(frame('health', { startId: 'a' }, 5))
    bridge.push(frame('health', { startId: 'b' }, 6))

    expect(first).toEqual(['a', 'b'])
    expect(second).toEqual(['a', 'b'])
    expect(other).toEqual([])
  })

  it('stops delivering after unsubscribe, also when a callback unsubscribes itself', () => {
    const bridge = createFakeBridge()
    const stream = createStream(bridge)
    const seen: string[] = []
    const stopFirst = stream.subscribe('health', () => {
      seen.push('first')
      stopFirst()
    })
    stream.subscribe('health', () => seen.push('second'))

    bridge.push(frame('health', {}))
    bridge.push(frame('health', {}))

    expect(seen).toEqual(['first', 'second', 'second'])
  })

  it('a subscriber that throws does not stop the others', () => {
    const bridge = createFakeBridge()
    const stream = createStream(bridge)
    const error = vi.spyOn(console, 'error').mockImplementation(() => {})
    const seen: string[] = []
    stream.subscribe('health', () => {
      throw new Error('broken screen')
    })
    stream.subscribe('health', () => seen.push('second'))

    bridge.push(frame('health', {}))

    expect(seen).toEqual(['second'])
    expect(error).toHaveBeenCalledOnce()
    error.mockRestore()
  })

  it('does not hand hello, resync or lifecycle frames to topic subscribers', () => {
    const bridge = createFakeBridge()
    const stream = createStream(bridge)
    const seen: unknown[] = []
    for (const topic of ['sessions', 'jobs', 'health'] as const) stream.subscribe(topic, (payload) => seen.push(payload))

    hello(bridge, 'start-1')
    resync(bridge)
    bridge.push(frame('lifecycle', { kind: 'stopping', reason: 'stop' }))

    expect(seen).toEqual([])
  })

  it('rewrites media references in topic payloads, except for scan frames', () => {
    const bridge = createFakeBridge()
    const stream = createStream(bridge)
    const launches: unknown[] = []
    const scans: unknown[] = []
    stream.subscribe('launch', (payload) => launches.push(payload))
    stream.subscribe('scan', (payload) => scans.push(payload))
    const payload = { steps: [{ icon: { kind: 'url', value: 'media/app-icon/x' } }] }

    bridge.push(frame('launch', structuredClone(payload)))
    bridge.push(frame('scan', structuredClone(payload)))

    expect(launches).toEqual([{ steps: [{ icon: { kind: 'url', value: `${FAKE_MEDIA_ORIGIN}media/app-icon/x` } }] }])
    expect(scans).toEqual([payload])
  })
})

describe('stream client: state', () => {
  it('follows the connection state of the bridge', () => {
    const bridge = createFakeBridge()
    const stream = createStream(bridge)

    bridge.setState('up')
    expect(stream.state.value).toBe('up')
    bridge.setState('down')
    expect(stream.state.value).toBe('down')
  })
})
