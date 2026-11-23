// Runtime plumbing shared by every mock slice: the topic bus, simulated latency,
// pausable timers, a seeded random generator and relative-time helpers.

import type { RainError, Topic, TopicPayload } from '../api'
import type { IconRef, ISODateTime } from '../types'
import { options } from './options'

// ---- time ----

export const SECOND = 1000
export const MINUTE = 60 * SECOND
export const HOUR = 60 * MINUTE
export const DAY = 24 * HOUR

/** A moment `ms` before now. Seeds use it so sample times stay plausible on any day. */
export function ago(ms: number): ISODateTime {
  return new Date(Date.now() - ms).toISOString()
}

export function fromNow(ms: number): ISODateTime {
  return new Date(Date.now() + ms).toISOString()
}

// ---- seeded random numbers ----

/** FNV-1a: a small, stable 32-bit hash of a string. */
export function hashString(text: string): number {
  let hash = 0x811c9dc5
  for (let i = 0; i < text.length; i += 1) {
    hash ^= text.charCodeAt(i)
    hash = Math.imul(hash, 0x01000193)
  }
  return hash >>> 0
}

/** mulberry32: a tiny generator that returns the same sequence for the same seed. */
export function mulberry32(seed: number): () => number {
  let a = seed >>> 0
  return () => {
    a = (a + 0x6d2b79f5) >>> 0
    let t = a
    t = Math.imul(t ^ (t >>> 15), t | 1)
    t ^= t + Math.imul(t ^ (t >>> 7), t | 61)
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296
  }
}

export interface Rng {
  /** A float in [0, 1). */
  next(): number
  /** An integer from min to max, both included. */
  int(min: number, max: number): number
  pick<T>(items: readonly T[]): T
  chance(probability: number): boolean
  /** A shuffled copy. */
  shuffle<T>(items: readonly T[]): T[]
}

/** A generator seeded by a name, so each data set has its own stable sequence. */
export function createRng(seed: string): Rng {
  const next = mulberry32(hashString(seed))
  const int = (min: number, max: number) => min + Math.floor(next() * (max - min + 1))
  return {
    next,
    int,
    pick: (items) => items[int(0, items.length - 1)],
    chance: (probability) => next() < probability,
    shuffle: (items) => {
      const copy = [...items]
      for (let i = copy.length - 1; i > 0; i -= 1) {
        const j = int(0, i)
        ;[copy[i], copy[j]] = [copy[j], copy[i]]
      }
      return copy
    },
  }
}

// ---- small data helpers ----

/** 'Web dev' -> 'web-dev'. Used for stable ids. */
export function slug(text: string): string {
  return text.toLowerCase().replace(/[^a-z0-9]+/g, '-').replace(/^-|-$/g, '')
}

/** Adds "-2", "-3", ... until the id is not in `taken`. */
export function uniqueId(base: string, taken: readonly string[]): string {
  let id = base
  for (let n = 2; taken.includes(id); n += 1) id = `${base}-${n}`
  return id
}

export const lucide = (value: string): IconRef => ({ kind: 'lucide', value })
export const brand = (value: string): IconRef => ({ kind: 'brand', value })
export const letter = (value: string): IconRef => ({ kind: 'letter', value })
export const image = (value: string): IconRef => ({ kind: 'url', value })

/** A deep copy. The API hands out copies so the UI can never change mock state by accident. */
export function clone<T>(value: T): T {
  return structuredClone(value)
}

// ---- errors ----

/** Builds the rejection value of an API call: `throw fail('notFound', 'Pool not found')`. */
export function fail(code: RainError['code'], message: string, source?: string): RainError {
  return source === undefined ? { code, message } : { code, message, source }
}

/** The item with this id, or a 'notFound' rejection that names what was looked for ('Pool not found'). */
export function findOrFail<T extends { id: string }>(items: readonly T[], id: string, what: string): T {
  const item = items.find((candidate) => candidate.id === id)
  if (!item) throw fail('notFound', `${what} not found`)
  return item
}

// ---- simulated latency ----

// Timing jitter has its own generator so it never disturbs the seeded data.
const jitter = mulberry32(0x5eed)

function between(min: number, max: number): number {
  return Math.round(min + jitter() * (max - min))
}

/**
 * Waits like a network call would. 'call' is 150 to 400 ms, or nothing when the
 * latency option is off. 'search' is always 0 to 30 ms.
 */
export function delay(kind: 'call' | 'search' = 'call'): Promise<void> {
  const ms = kind === 'search' ? between(0, 30) : options.latency ? between(150, 400) : 0
  if (ms === 0) return Promise.resolve()
  return new Promise((resolve) => setTimeout(resolve, ms))
}

/** How long one launch step takes. */
export function stepDuration(): number {
  return between(400, 900)
}

// ---- topic bus ----

type Listener<T extends Topic> = (payload: TopicPayload[T]) => void

const listeners: { [T in Topic]: Set<Listener<T>> } = {
  sessions: new Set(),
  jobs: new Set(),
  idleJobs: new Set(),
  launch: new Set(),
  scan: new Set(),
  gameMode: new Set(),
  // The mock never publishes these two; the router sends them to the backend.
  settings: new Set(),
  health: new Set(),
}

export function subscribe<T extends Topic>(topic: T, cb: Listener<T>): () => void {
  const set: Set<Listener<T>> = listeners[topic]
  set.add(cb)
  return () => {
    set.delete(cb)
  }
}

export function emit<T extends Topic>(topic: T, payload: TopicPayload[T]): void {
  const set: Set<Listener<T>> = listeners[topic]
  for (const cb of [...set]) cb(payload)
}

// ---- pausable timers ----
// Everything that moves on its own in the mock (launch steps, scans, the game-mode
// countdown, session activity) runs on these timers. Pausing freezes them all, so
// nothing ticks while the window is hidden.

interface Timer {
  run: () => void
  /** Repeat interval, or null for a one-shot timer. */
  everyMs: number | null
  remainingMs: number
  dueAt: number
  handle?: ReturnType<typeof setTimeout>
}

const timers = new Set<Timer>()
let paused = false

function arm(timer: Timer): void {
  timer.dueAt = Date.now() + timer.remainingMs
  timer.handle = setTimeout(() => fire(timer), timer.remainingMs)
}

function fire(timer: Timer): void {
  if (timer.everyMs === null) {
    timers.delete(timer)
  } else {
    timer.remainingMs = timer.everyMs
    arm(timer)
  }
  timer.run()
}

function start(ms: number, everyMs: number | null, run: () => void): () => void {
  const timer: Timer = { run, everyMs, remainingMs: ms, dueAt: 0 }
  timers.add(timer)
  if (!paused) arm(timer)
  return () => {
    clearTimeout(timer.handle)
    timers.delete(timer)
  }
}

/** Runs `run` once after `ms`. Returns a function that cancels it. */
export function after(ms: number, run: () => void): () => void {
  return start(ms, null, run)
}

/** Runs `run` every `ms`. Returns a function that stops it. */
export function every(ms: number, run: () => void): () => void {
  return start(ms, ms, run)
}

/** Freezes or resumes every timer. A resumed timer continues with the time it had left. */
export function setPaused(next: boolean): void {
  if (next === paused) return
  paused = next
  for (const timer of timers) {
    if (paused) {
      clearTimeout(timer.handle)
      timer.remainingMs = Math.max(0, timer.dueAt - Date.now())
    } else {
      arm(timer)
    }
  }
}

/** Drops every timer. Called before the data is reseeded, so no old process writes into new data. */
export function cancelTimers(): void {
  for (const timer of timers) clearTimeout(timer.handle)
  timers.clear()
}
