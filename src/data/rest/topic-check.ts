// Compile-time check, no runtime code: the topics of `RainApi` (hand-written in api.ts) must be
// exactly the topics the backend publishes (TOPIC_NAMES, generated from Rust). A topic that
// exists on one side only makes this file fail to compile (foundation 9.3).
import type { TOPIC_NAMES } from '@/bindings/rain-types'
import type { Topic } from '../api'

type Equal<A, B> = [A] extends [B] ? ([B] extends [A] ? true : false) : false
type MustBeTrue<T extends true> = T

export type TopicsMatchBackend = MustBeTrue<Equal<Topic, (typeof TOPIC_NAMES)[number]>>
