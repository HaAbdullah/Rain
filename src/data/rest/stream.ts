// The live stream (foundation 5 and 8.7). One stream is opened for the life of the page and
// every `subscribe` is a callback on one of its topics. The stream never says what was missed:
// after every (re)connect it sends `resync`, and the page refetches what it shows.
import { readonly, ref, type Ref } from 'vue'
import type { Bridge, BackendState, StreamFrame } from '@/shell/bridge'
import type { Topic, TopicPayload } from '../api'
import { resolveMedia } from './media'

export interface StreamResetInfo {
  /** True when the backend process is not the one the page last talked to (it was restarted). */
  restarted: boolean
}

export interface RestStream {
  /** Same contract as `RainApi.subscribe`: synchronous, returns the unsubscribe function. */
  subscribe<T extends Topic>(topic: T, cb: (payload: TopicPayload[T]) => void): () => void
  /** Runs after every `resync`: the page must refetch what it shows. Returns the unsubscribe function. */
  onReset(cb: (info: StreamResetInfo) => void): () => void
  /** What the connection to the backend is doing. For a future banner. */
  state: Readonly<Ref<BackendState>>
}

type Subscriber = (payload: unknown) => void

/** Topics whose payloads are large and have no images, so they skip the media rewrite. */
const NO_MEDIA_TOPICS = new Set<string>(['scan'])

function isStartId(payload: unknown): payload is { startId: string } {
  return typeof payload === 'object' && payload !== null && typeof (payload as { startId?: unknown }).startId === 'string'
}

/**
 * Opens the bridge's stream at once (not at the first `subscribe`), so the first `hello`,
 * which decides whether the backend restarted, can never be missed.
 */
export function createStream(bridge: Bridge): RestStream {
  const state = ref<BackendState>('starting')
  const subscribers = new Map<string, Set<Subscriber>>()
  const resetListeners = new Set<(info: StreamResetInfo) => void>()

  let lastStartId: string | undefined
  let restarted = false

  function deliver(frame: StreamFrame) {
    const listeners = subscribers.get(frame.topic)
    if (!listeners) return
    const payload = NO_MEDIA_TOPICS.has(frame.topic)
      ? frame.payload
      : resolveMedia(frame.payload, bridge.mediaUrl)
    // Copy first: a callback may unsubscribe itself while it runs. One that throws must not
    // starve the others.
    for (const cb of [...listeners]) {
      try {
        cb(payload)
      } catch (error) {
        console.error(`A subscriber of "${frame.topic}" threw`, error)
      }
    }
  }

  function onFrame(frame: StreamFrame) {
    switch (frame.topic) {
      case 'hello':
        // The first hello has nothing to compare with, so it is never a restart.
        if (isStartId(frame.payload)) {
          restarted = lastStartId !== undefined && lastStartId !== frame.payload.startId
          lastStartId = frame.payload.startId
        }
        break
      case 'resync':
        // `hello` comes first on a connect; on a lag there is no hello and `restarted` is false.
        for (const cb of [...resetListeners]) cb({ restarted })
        restarted = false
        break
      case 'lifecycle':
        // Only meaningful to the shell, which restarts the backend; the page just waits for the next hello.
        break
      default:
        deliver(frame)
    }
  }

  bridge.openStream({ onFrame, onState: (next) => (state.value = next) })

  return {
    subscribe(topic, cb) {
      const listeners = subscribers.get(topic) ?? new Set<Subscriber>()
      subscribers.set(topic, listeners)
      listeners.add(cb as Subscriber)
      return () => listeners.delete(cb as Subscriber)
    },
    onReset(cb) {
      resetListeners.add(cb)
      return () => resetListeners.delete(cb)
    },
    state: readonly(state),
  }
}
