// Strict type check between the hand-written TypeScript contract (src/data) and the file that
// Rust generates (src/bindings/rain-types.ts). Run it with `npm run types:identity`.
//
// This is a compile-time check only: it fails by making `tsc` report an error. Nothing runs.
// `Equals` is true only when two types are exactly the same, not merely assignable to each other.
//
// Stage 1a covers the types that exist on both sides. When a later stage moves a type into
// `src/bindings/rain-types.ts` and deletes the hand-written one, its line here is replaced by a
// check of its own (or removed once `src/data/types.ts` only re-exports the generated file).

import type * as G from '../../../src/bindings/rain-types'
import type { RainError, Topic } from '../../../src/data/api'
import type * as H from '../../../src/data/types'

type Equals<A, B> =
  (<T>() => T extends A ? 1 : 2) extends (<T>() => T extends B ? 1 : 2) ? true : false

// ---- types that are the same on both sides ----
export const eqStatus: Equals<H.Status, G.Status> = true
export const eqIconRef: Equals<H.IconRef, G.IconRef> = true
// The three hard cases are in `Settings`: a number union, optional fields and arrays.
export const eqSettings: Equals<H.Settings, G.Settings> = true

// A patch holds the settings the user may change: all optional, without the read-only facts.
type ReadOnlySetting = 'hotkeyAvailable' | 'calendarConnected'
export const eqSettingsPatch: Equals<Partial<Omit<H.Settings, ReadOnlySetting>>, G.SettingsPatch> =
  true

// ---- the error body grew on purpose (foundation section 15, row F1) ----
// Every old error is still a valid new one, and the new one adds two codes and `reason`.
export const oldErrorIsStillValid: RainError extends G.ApiError ? true : false = true
export const errorCodesGrew: Equals<G.ApiError['code'], RainError['code'] | 'invalid' | 'conflict'> =
  true
export const errorCodeAliasMatches: Equals<G.ErrorCode, G.ApiError['code']> = true

// ---- the topic names ----
// `health` and `settings` are real topics from stage 1b; the six old names must still agree.
// (The frontend adds the two new names to its own `Topic` in the same stage; both sides are
// compared without them so that this check does not depend on the order of those edits.)
type NewTopic = 'health' | 'settings'
export const topicNamesMatch: Equals<
  Exclude<(typeof G.TOPIC_NAMES)[number], NewTopic>,
  Exclude<Topic, NewTopic>
> = true

// ---- JSON that the backend sends must be assignable to the generated types ----
export const wirePing = {
  app: 'rain-backend', version: '0.1.0', apiVersion: 1, startId: 'abc', pid: 1,
  exePath: 'C:\\rain-backend.exe', sandbox: true, busy: false,
} satisfies G.Ping
export const wireHealth = {
  startId: 'abc', version: '0.1.0', shell: { reported: false },
  configIssues: [{ file: 'settings.toml', message: 'bad' }],
  sources: [{ name: 'steam', state: 'off' }],
} satisfies G.Health
export const wireFrame = { topic: 'health', seq: 1, payload: wireHealth } satisfies G.Frame
export const wireHello = { startId: 'abc', version: '0.1.0', apiVersion: 1, seq: 0 } satisfies G.Hello
export const wireResync = { reason: 'lagged', missed: 3 } satisfies G.Resync
export const wireLifecycle = { kind: 'stopping', reason: 'gameMode' } satisfies G.Lifecycle
export const wireError = { code: 'conflict', message: 'Busy', reason: 'busy' } satisfies G.ApiError
export const wirePatch = { resetAfterSeconds: 300, hideOnBlur: false } satisfies G.SettingsPatch

// ---- negative controls: the check must be able to fail ----
export const differentTypesAreNotEqual: Equals<H.Settings, G.Ping> = false
export const optionalIsNotRequired: Equals<{ a?: string }, { a: string }> = false
export const unionIsNotPlainNumber: Equals<G.ResetAfterSeconds, number> = false
// @ts-expect-error 45 is not in the union 30 | 60 | 300
export const badNumberIsRejected: G.SettingsPatch = { resetAfterSeconds: 45 }
// @ts-expect-error a read-only setting cannot be patched
export const readOnlyIsRejected: G.SettingsPatch = { hotkeyAvailable: true }
// @ts-expect-error `null` is not allowed for an optional field
export const nullIsRejected: G.ApiError = { code: 'failed', message: 'x', reason: null }
