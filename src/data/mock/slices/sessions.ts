// Mock slice: Claude sessions. Answers to permission requests change the
// session, and while the window is visible one working session reports new
// activity every 20 seconds. Every change is sent on the 'sessions' topic.

import type { RainApi } from '../../api'
import type { ClaudeSession, SessionEvent } from '../../types'
import { SECOND, clone, createRng, delay, emit, every, fail, findOrFail } from '../engine'
import { sessionTranscript } from '../logs'
import { REQUEST_SAMPLES, sampleActivity } from '../seed/workflows'
import { nextId, state } from '../store'

const MAX_RECENT_EVENTS = 8
const ACTIVITY_EVERY_MS = 20 * SECOND

const rng = createRng('session-activity')

type SessionsSlice = Pick<RainApi,
  'listSessions' | 'answerPermission' | 'focusSession' | 'stopSession' | 'getSessionTranscript'>

export const sessionsSlice: SessionsSlice = {
  async listSessions() {
    await delay()
    return clone(state.sessions)
  },

  async answerPermission(sessionId, answer) {
    await delay()
    const session = findOrFail(state.sessions, sessionId, 'Session')
    const request = session.request
    if (!request) throw fail('failed', 'This session is not waiting for an answer')

    const what = `${request.tool}: ${request.summary}`
    session.request = undefined
    if (answer === 'deny') {
      session.status = 'idle'
      session.lastMessage = 'Permission denied. Tell me how you want to continue.'
      addEvent(session, 'message', `Denied ${what}`)
    } else {
      session.status = 'running'
      addEvent(session, 'tool', answer === 'always' ? `Always allowed ${what}` : `Allowed ${what}`)
    }
    publish()
    return clone(session)
  },

  async focusSession(id) {
    await delay()
    findOrFail(state.sessions, id, 'Session')
  },

  async stopSession(id) {
    await delay()
    const session = findOrFail(state.sessions, id, 'Session')
    session.status = 'idle'
    session.request = undefined
    addEvent(session, 'message', 'Stopped from Rain')
    publish()
  },

  async getSessionTranscript(id) {
    await delay()
    return sessionTranscript(findOrFail(state.sessions, id, 'Session'))
  },
}

/** Starts the background activity. Called once at start-up and again after every reseed. */
export function startSessionActivity(): void {
  every(ACTIVITY_EVERY_MS, () => {
    const working = workingSessions()
    if (working.length === 0) return
    // The longest-silent session goes first. One event per tick keeps two sessions
    // fresh; with more, half of them report, or the UI would mark live sessions as
    // stale (silent for 60 s).
    const longestSilentFirst = [...working].sort((a, b) => a.lastActivityAt.localeCompare(b.lastActivityAt))
    for (const session of longestSilentFirst.slice(0, Math.ceil(working.length / 2))) {
      const activity = sampleActivity(rng)
      addEvent(session, activity.kind, activity.text)
      session.tokens += rng.int(200, 1800)
    }
    publish()
  })
}

/** The sessions kept working while the window was hidden, so none looks stale on the next summon. */
export function markWorkingSessionsActive(): void {
  const at = new Date().toISOString()
  for (const session of workingSessions()) session.lastActivityAt = at
  publish()
}

/** Prototype control: one session (a working one if there is any) now waits for permission. */
export function requestPermission(): void {
  const session = workingSessions()[0] ?? state.sessions.find((candidate) => candidate.status !== 'waiting')
  if (!session) return

  const sample = rng.pick(REQUEST_SAMPLES)
  session.status = 'waiting'
  session.request = { id: nextId('req'), ...sample, requestedAt: new Date().toISOString() }
  session.lastMessage = `I need your permission to use ${sample.tool}.`
  addEvent(session, 'permission', `${sample.tool}: ${sample.summary}`)
  publish()
}

/** Game mode "Wait for them": the working sessions finish their task. Returns how many did. */
export function finishWorkingSessions(): number {
  const working = state.sessions.filter((session) => session.status === 'running')
  for (const session of working) {
    session.status = 'success'
    session.lastMessage = 'Done.'
    addEvent(session, 'message', 'Done.')
  }
  if (working.length > 0) publish()
  return working.length
}

/** Working sessions that still report activity (the quiet ones are the stale examples). */
function workingSessions(): ClaudeSession[] {
  return state.sessions.filter((session) => session.status === 'running' && !state.quietSessionIds.includes(session.id))
}

function addEvent(session: ClaudeSession, kind: SessionEvent['kind'], text: string): void {
  const at = new Date().toISOString()
  session.recent.unshift({ at, kind, text })
  session.recent.length = Math.min(session.recent.length, MAX_RECENT_EVENTS)
  session.lastActivityAt = at
}

function publish(): void {
  emit('sessions', clone(state.sessions))
}
