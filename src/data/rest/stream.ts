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
  /**
   * Hands a state that an API call returned to the subscribers of its topic, as a frame would,
   * unless it is late (see `isLate`). Returns the payload either way. Section 06 6.10: the answer
   * of `startGameMode` and `resolveGameMode` reaches the screen before the call resolves.
   */
  deliver<T extends Topic>(topic: T, payload: TopicPayload[T]): TopicPayload[T]
  /** Raises the mark of `topic` without delivering (for `getGameMode`, which is never dropped). */
  note<T extends Topic>(topic: T, payload: TopicPayload[T]): TopicPayload[T]
  /** What the connection to the backend is doing. For a future banner. */
  state: Readonly<Ref<BackendState>>
}

type Subscriber = (payload: unknown) => void

/** Topics whose payloads are large and have no images, so they skip the media rewrite. */
const NO_MEDIA_TOPICS = new Set<string>(['scan'])

/** Topics whose payload carries a `rev` that only grows within one backend process. */
const REV_TOPICS = new Set<string>(['gameMode'])

function revOf(payload: unknown): number | undefined {
  const rev = typeof payload === 'object' && payload !== null ? (payload as { rev?: unknown }).rev : undefined
  return typeof rev === 'number' ? rev : undefined
}

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
  /** The highest `rev` applied for each topic with revisions, since the last new backend. */
  const highestRev = new Map<string, number>()

  /**
   * A response and a pushed frame travel on different channels, so a frame can arrive after
   * the response and after the screen has left. A state whose `rev` is not above the highest
   * applied is late and is dropped. A payload without a `rev` is never late.
   */
  function isLate(topic: string, payload: unknown): boolean {
    if (!REV_TOPICS.has(topic)) return false
    const rev = revOf(payload)
    if (rev === undefined) return false
    const highest = highestRev.get(topic) ?? 0
    if (rev <= highest) return true
    highestRev.set(topic, rev)
    return false
  }

  function raiseMark(topic: string, payload: unknown) {
    const rev = revOf(payload)
    if (REV_TOPICS.has(topic) && rev !== undefined && rev > (highestRev.get(topic) ?? 0)) highestRev.set(topic, rev)
  }

  /** Gives a frame to the subscribers of its topic. The lateness check has been made by the caller. */
  function fanOut(frame: StreamFrame) {
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
          // A new process starts its `rev` at 1 again.
          if (restarted) highestRev.clear()
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
        // The mark is raised even when nobody listens yet, so a later listener never sees an older state.
        if (!isLate(frame.topic, frame.payload)) fanOut(frame)
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
    deliver(topic, payload) {
      if (!isLate(topic, payload)) fanOut({ topic, seq: 0, payload })
      return payload
    },
    note(topic, payload) {
      raiseMark(topic, payload)
      return payload
    },
    onReset(cb) {
      resetListeners.add(cb)
      return () => resetListeners.delete(cb)
    },
    state: readonly(state),
  }
}
