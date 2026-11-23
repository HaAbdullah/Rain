// The five Claude session methods of the REST client (section 03 9.1): method, path, body and the
// answer, with the generated types.
import { describe, expect, it } from 'vitest'
import type { ClaudeSession, FocusResult, LogLine, PermissionAnswerResult } from '../types'
import { createRestApi } from './client'
import { createFakeBridge } from './fake-bridge.testing'

function setup() {
  const bridge = createFakeBridge()
  const api = createRestApi(bridge, { subscribe: () => () => {} })
  return { bridge, api }
}

const SESSION: ClaudeSession = {
  id: 'aaaaaaaa-0000-0000-0000-000000000001',
  title: 'Fix the parser',
  cwd: 'C:\\work\\demo',
  model: 'sonnet 4.5',
  status: 'waiting',
  startedAt: '2026-11-21T09:00:00.000Z',
  lastActivityAt: '2026-11-21T09:01:00.000Z',
  tokens: 36_000,
  lastMessage: 'I need to run the tests.',
  request: {
    id: 'req-0a1b2c3d',
    tool: 'Bash',
    summary: 'npm install',
    detail: 'npm install',
    requestedAt: '2026-11-21T09:01:00.000Z',
    answerable: true,
  },
  recent: [],
  host: 'terminal',
  alive: true,
}

describe('REST client: sessions', () => {
  it('listSessions is GET /sessions', async () => {
    const { bridge, api } = setup()
    bridge.answerWith([SESSION])

    expect(await api.listSessions()).toEqual([SESSION])
    expect(bridge.calls).toEqual([{ method: 'GET', path: '/sessions', body: undefined }])
  })

  it('answerPermission is POST /sessions/:id/permission with the request id and the answer', async () => {
    const { bridge, api } = setup()
    const result: PermissionAnswerResult = { applied: 'always', ruleText: 'Bash(npm install)' }
    bridge.answerWith(result)

    expect(await api.answerPermission(SESSION.id, 'req-0a1b2c3d', 'always')).toEqual(result)
    expect(bridge.calls).toEqual([
      {
        method: 'POST',
        path: `/sessions/${SESSION.id}/permission`,
        body: { requestId: 'req-0a1b2c3d', answer: 'always' },
      },
    ])
  })

  it('focusSession is POST /sessions/:id/focus and returns what happened', async () => {
    const { bridge, api } = setup()
    const result: FocusResult = { focused: 'none' }
    bridge.answerWith(result)

    expect(await api.focusSession('s 1')).toEqual(result)
    expect(bridge.calls).toEqual([{ method: 'POST', path: '/sessions/s%201/focus', body: undefined }])
  })

  it('stopSession is POST /sessions/:id/stop and resolves to nothing', async () => {
    const { bridge, api } = setup()
    bridge.answerWith(undefined)

    expect(await api.stopSession(SESSION.id)).toBeUndefined()
    expect(bridge.calls).toEqual([{ method: 'POST', path: `/sessions/${SESSION.id}/stop`, body: undefined }])
  })

  it('getSessionTranscript is GET /sessions/:id/transcript and leaves the text alone', async () => {
    const { bridge, api } = setup()
    // A line may quote a path that looks like a media reference: it must not be rewritten.
    const lines: LogLine[] = [{ ts: '2026-11-21T09:00:00.000Z', level: 'info', text: 'media/app-icon/x?v=1' }]
    bridge.answerWith(lines)

    expect(await api.getSessionTranscript(SESSION.id)).toEqual(lines)
    expect(bridge.calls).toEqual([{ method: 'GET', path: `/sessions/${SESSION.id}/transcript`, body: undefined }])
  })

  it('an expired request is the error of the backend, unchanged', async () => {
    const { bridge, api } = setup()
    const error = {
      code: 'conflict',
      message: 'That request is no longer open. Answer it in the terminal.',
      reason: 'requestExpired',
    }
    bridge.failWith(error)

    await expect(api.answerPermission(SESSION.id, 'req-x', 'once')).rejects.toEqual(error)
  })
})
