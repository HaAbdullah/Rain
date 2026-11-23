import { describe, expect, it } from 'vitest'
import type { Health, Settings } from '../types'
import { createRestApi } from './client'
import { createFakeBridge } from './fake-bridge'

const SETTINGS: Settings = {
  hotkey: 'Ctrl+Backquote',
  hotkeyAvailable: true,
  hideOnBlur: true,
  resetAfterSeconds: 60,
  startWithWindows: false,
  dataDir: '',
  vaultPath: '',
  dailyNoteFormat: 'YYYY-MM-DD',
  dailyNoteFolder: '',
  projectRoots: [],
  gameFolders: [],
  steamEnabled: true,
  steamPath: '',
  epicEnabled: true,
  epicPath: '',
  calendarConnected: false,
  monitorClaude: false,
  monitorClaudeState: 'off',
  gameModeCountdownSeconds: 10,
}

function setup() {
  const bridge = createFakeBridge()
  const api = createRestApi(bridge, { subscribe: () => () => {} })
  return { bridge, api }
}

describe('REST client: settings', () => {
  it('getSettings is GET /settings with no body and returns the answer', async () => {
    const { bridge, api } = setup()
    bridge.answerWith(SETTINGS)

    expect(await api.getSettings()).toEqual(SETTINGS)
    expect(bridge.calls).toEqual([{ method: 'GET', path: '/settings', body: undefined }])
  })

  it('updateSettings is PATCH /settings with the patch as the body', async () => {
    const { bridge, api } = setup()
    bridge.answerWith({ ...SETTINGS, resetAfterSeconds: 300 })

    const saved = await api.updateSettings({ resetAfterSeconds: 300 })

    expect(saved.resetAfterSeconds).toBe(300)
    expect(bridge.calls).toEqual([{ method: 'PATCH', path: '/settings', body: { resetAfterSeconds: 300 } }])
  })
})

describe('REST client: errors', () => {
  it('passes the backend error body through unchanged', async () => {
    const { bridge, api } = setup()
    const error = { code: 'invalid', message: 'The request was not valid', source: 'settings', reason: 'badBody' }
    bridge.failWith(error)

    await expect(api.getSettings()).rejects.toEqual(error)
  })

  it('turns anything else that is thrown into a failed RainError', async () => {
    const { bridge, api } = setup()
    bridge.failWith(new Error('socket closed'))

    await expect(api.getSettings()).rejects.toEqual({ code: 'failed', message: 'socket closed' })
  })

  it('does not retry: one failed call is one request', async () => {
    const { bridge, api } = setup()
    bridge.failWith({ code: 'unavailable', message: 'Rain backend is not running' })

    await api.getSettings().catch(() => {})
    expect(bridge.calls).toHaveLength(1)
  })
})

describe('REST client: methods that are not built yet', () => {
  it('rejects with the notImplemented error, and sends nothing', async () => {
    const { bridge, api } = setup()

    await expect(api.listApps()).rejects.toMatchObject({
      code: 'unavailable',
      source: 'client',
      reason: 'notImplemented',
    })
    expect(bridge.calls).toEqual([])
  })
})

describe('REST client: media', () => {
  it('rewrites media references in an answer to loadable addresses', async () => {
    const { bridge, api } = setup()
    // No implemented method returns images yet, so the rewrite is seen through getSettings.
    bridge.answerWith({ coverUrl: 'media/game-cover/g1?v=2', icon: { kind: 'url', value: 'media/app-icon/a?v=1' } })

    expect(await api.getSettings()).toEqual({
      coverUrl: 'test://media-host/media/game-cover/g1?v=2',
      icon: { kind: 'url', value: 'test://media-host/media/app-icon/a?v=1' },
    })
  })
})

describe('REST client: getHealth', () => {
  it('asks GET /health and returns the answer', async () => {
    const { bridge, api } = setup()
    const health = {
      startId: 's1',
      version: '0.1.0',
      shell: { reported: true, hotkeyRegistered: true },
      configIssues: [{ file: 'settings.toml', line: 4, message: 'expected a value' }],
      sources: [{ name: 'steam', state: 'off', message: 'not installed' }],
    } satisfies Health
    bridge.answerWith(health)

    expect(await api.getHealth()).toEqual(health)
    expect(bridge.calls).toEqual([{ method: 'GET', path: '/health', body: undefined }])
  })
})
