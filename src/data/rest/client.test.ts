import { describe, expect, it } from 'vitest'
import type { Health, IdleJob, LogLine, Project, ScheduledJob, Settings } from '../types'
import { createRestApi } from './client'
import { createFakeBridge } from './fake-bridge.testing'

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
  riotEnabled: true,
  riotPath: '',
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

describe('REST client: disk and machine', () => {
  it('sends each call to its route with the right method, path and body', async () => {
    const { bridge, api } = setup()
    await api.listDrives()
    await api.startScan('drive-c')
    await api.getScan('scan-1')
    await api.getScanNode('scan-1', 'C:\\Users')
    await api.cancelScan('scan-1')
    await api.revealPath('C:\\Users\\x.txt')
    await api.checkPaths(['C:\\a', 'D:\\b'])
    await api.getMachineState()
    expect(bridge.calls).toEqual([
      { method: 'GET', path: '/drives', body: undefined },
      { method: 'POST', path: '/scans', body: { driveId: 'drive-c' } },
      { method: 'GET', path: '/scans/scan-1', body: undefined },
      { method: 'GET', path: '/scans/scan-1/node?path=C%3A%5CUsers', body: undefined },
      { method: 'DELETE', path: '/scans/scan-1', body: undefined },
      { method: 'POST', path: '/shell/reveal', body: { path: 'C:\\Users\\x.txt' } },
      { method: 'POST', path: '/paths/check', body: { paths: ['C:\\a', 'D:\\b'] } },
      { method: 'GET', path: '/machine', body: undefined },
    ])
  })

  it('does not rewrite the answer of a scan (it holds no media and can be large)', async () => {
    const { bridge, api } = setup()
    const scan = { id: 's', driveId: 'd', status: 'done', progress: 1, filesSeen: 1, root: { coverUrl: 'media/x/y' } }
    bridge.answerWith(scan)
    expect(await api.getScan('s')).toEqual(scan)
  })
})

describe('REST client: launcher and apps', () => {
  it('search encodes the text and sends the types as a comma list', async () => {
    const { bridge, api } = setup()
    bridge.answerWith([])

    await api.search('dev web & more', ['app', 'pool'])
    await api.search('x')

    expect(bridge.calls).toEqual([
      { method: 'GET', path: '/search?q=dev%20web%20%26%20more&types=app%2Cpool', body: undefined },
      { method: 'GET', path: '/search?q=x', body: undefined },
    ])
  })

  it('reads the lists with plain GET calls and the recent limit as a query', async () => {
    const { bridge, api } = setup()
    bridge.answerWith([])

    await api.listPinned()
    await api.listRecent(12)
    await api.listApps()
    await api.listPools()

    expect(bridge.calls.map((call) => `${call.method} ${call.path}`)).toEqual([
      'GET /pinned',
      'GET /recent?limit=12',
      'GET /apps',
      'GET /pools',
    ])
  })

  it('setPinned is PUT /pinned/{type}/{id} with the flag as the body', async () => {
    const { bridge, api } = setup()
    bridge.answerWith(undefined)

    await api.setPinned('pool', 'pool-web dev', true)

    expect(bridge.calls).toEqual([{ method: 'PUT', path: '/pinned/pool/pool-web%20dev', body: { pinned: true } }])
  })

  it('rescanApps is POST /apps/scan', async () => {
    const { bridge, api } = setup()
    bridge.answerWith([])

    await api.rescanApps()

    expect(bridge.calls).toEqual([{ method: 'POST', path: '/apps/scan', body: undefined }])
  })

  it('savePool creates with POST when the draft has no id and updates with PUT when it has one', async () => {
    const { bridge, api } = setup()
    bridge.answerWith({})
    const members = [{ appId: 'app-a' }]

    await api.savePool({ name: 'New', members })
    await api.savePool({ id: 'pool-x', name: 'Old', members })

    expect(bridge.calls).toEqual([
      { method: 'POST', path: '/pools', body: { name: 'New', members } },
      { method: 'PUT', path: '/pools/pool-x', body: { id: 'pool-x', name: 'Old', members } },
    ])
  })

  it('deletePool, launch and getLaunch use the routes of the registry', async () => {
    const { bridge, api } = setup()
    bridge.answerWith({})

    await api.deletePool('pool-x')
    await api.launch({ type: 'app', id: 'app-a', args: '--new-window' })
    await api.getLaunch('launch-1-0')

    expect(bridge.calls).toEqual([
      { method: 'DELETE', path: '/pools/pool-x', body: undefined },
      { method: 'POST', path: '/launches', body: { type: 'app', id: 'app-a', args: '--new-window' } },
      { method: 'GET', path: '/launches/launch-1-0', body: undefined },
    ])
  })

  it('makes the media references of an app icon loadable', async () => {
    const { bridge, api } = setup()
    bridge.answerWith([
      { id: 'app-a', name: 'A', kind: 'program', keywords: [], pinned: false, running: false,
        icon: { kind: 'url', value: 'media/app-icon/app-a?v=3fa9c1d2', fallback: 'A' } },
    ])

    const apps = await api.listApps()

    expect(apps[0].icon.value).toBe('test://media-host/media/app-icon/app-a?v=3fa9c1d2')
    expect(apps[0].icon.fallback).toBe('A')
  })
})

describe('REST client: projects', () => {
  it('listProjects is GET /projects and returns the list as it is', async () => {
    const { bridge, api } = setup()
    const projects = [
      {
        id: 'proj-alpha',
        name: 'alpha',
        path: 'D:\\Work\\alpha',
        icon: { kind: 'letter', value: 'alpha' },
        branch: 'main',
        commits14d: new Array<number>(14).fill(0),
        activityScore: 0,
        environment: [],
        open: false,
        pinned: false,
        missing: false,
      },
    ] satisfies Project[]
    bridge.answerWith(projects)

    expect(await api.listProjects()).toEqual(projects)
    expect(bridge.calls).toEqual([{ method: 'GET', path: '/projects', body: undefined }])
  })
})

describe('REST client: games (section 02)', () => {
  it('listGames is GET /games and the cover references become loadable addresses', async () => {
    const { bridge, api } = setup()
    bridge.answerWith({
      games: [{ id: 'game-steam-1', title: 'A', source: 'steam', coverUrl: 'media/game-cover/game-steam-1?v=ab' }],
      sources: [
        { source: 'epic', state: 'off', message: 'Epic Games is not installed' },
        { source: 'riot', state: 'ok' },
      ],
    })

    const library = await api.listGames()

    expect(library.games[0].coverUrl).toBe('test://media-host/media/game-cover/game-steam-1?v=ab')
    expect(library.sources[0].state).toBe('off')
    expect(library.sources[1].source).toBe('riot')
    expect(bridge.calls).toEqual([{ method: 'GET', path: '/games', body: undefined }])
  })

  it('saveCustomGame is POST /games with the draft', async () => {
    const { bridge, api } = setup()
    bridge.answerWith({ id: 'game-celeste' })
    await api.saveCustomGame({ title: 'Celeste', exePath: 'D:\Games\Celeste\Celeste.exe' })
    expect(bridge.calls).toEqual([
      { method: 'POST', path: '/games', body: { title: 'Celeste', exePath: 'D:\Games\Celeste\Celeste.exe' } },
    ])
  })

  it('setGameHidden sends only the flag and returns nothing; updateGame sends the whole patch', async () => {
    const { bridge, api } = setup()
    bridge.answerWith({ id: 'game-x', hidden: true })

    expect(await api.setGameHidden('game-x', true)).toBeUndefined()
    const updated = await api.updateGame('game-x', { title: 'New', exePath: '' })

    expect(updated).toEqual({ id: 'game-x', hidden: true })
    expect(bridge.calls).toEqual([
      { method: 'PATCH', path: '/games/game-x', body: { hidden: true } },
      { method: 'PATCH', path: '/games/game-x', body: { title: 'New', exePath: '' } },
    ])
  })

  it('deleteCustomGame is DELETE /games/:id, with the id escaped', async () => {
    const { bridge, api } = setup()
    await api.deleteCustomGame('game-a b')
    expect(bridge.calls).toEqual([{ method: 'DELETE', path: '/games/game-a%20b', body: undefined }])
  })
})

describe('REST client: jobs and idle jobs (section 03)', () => {
  const job = {
    id: 'job-echo',
    source: 'rain',
    name: 'Say hello',
    kind: 'cron',
    cron: '0 6 * * *',
    scheduleLabel: 'Every day at 06:00',
    enabled: true,
    running: false,
    recentRuns: [],
  } satisfies ScheduledJob
  const idleJob = {
    id: 'idle-a',
    name: 'A',
    enabled: true,
    state: 'queued',
    progress: 0,
    resource: 'cpu',
    idleAfterMinutes: 10,
  } satisfies IdleJob
  const lines = [{ ts: '2026-11-21T09:30:00.000Z', level: 'info', text: 'hello' }] satisfies LogLine[]

  it('listJobs is GET /jobs', async () => {
    const { bridge, api } = setup()
    bridge.answerWith([job])
    expect(await api.listJobs()).toEqual([job])
    expect(bridge.calls).toEqual([{ method: 'GET', path: '/jobs', body: undefined }])
  })

  it('setJobEnabled is PATCH /jobs/{id} with the new value', async () => {
    const { bridge, api } = setup()
    bridge.answerWith({ ...job, enabled: false })
    expect((await api.setJobEnabled('job-echo', false)).enabled).toBe(false)
    expect(bridge.calls).toEqual([{ method: 'PATCH', path: '/jobs/job-echo', body: { enabled: false } }])
  })

  it('runJob is POST /jobs/{id}/runs and returns the run', async () => {
    const { bridge, api } = setup()
    const run = { id: 'run-1', jobId: 'job-echo', status: 'running', startedAt: '2026-11-21T09:30:00.000Z' }
    bridge.answerWith(run)
    expect(await api.runJob('job-echo')).toEqual(run)
    expect(bridge.calls).toEqual([{ method: 'POST', path: '/jobs/job-echo/runs', body: undefined }])
  })

  it('getRunLog is GET /runs/{id}/log and leaves the text alone (media: false)', async () => {
    const { bridge, api } = setup()
    // A log line may look like a media reference; it must not be rewritten.
    const odd = [{ ts: 't', level: 'info', text: 'media/app-icon/x?v=1' }]
    bridge.answerWith(odd)
    expect(await api.getRunLog('run-1')).toEqual(odd)
    expect(bridge.calls).toEqual([{ method: 'GET', path: '/runs/run-1/log', body: undefined }])
  })

  it('listIdleJobs, setIdleJobEnabled and runIdleJob use the routes of the idle jobs', async () => {
    const { bridge, api } = setup()
    bridge.answerWith([idleJob])
    await api.listIdleJobs()
    bridge.answerWith(idleJob)
    await api.setIdleJobEnabled('idle-a', true)
    await api.runIdleJob('idle-a')
    expect(bridge.calls).toEqual([
      { method: 'GET', path: '/idle-jobs', body: undefined },
      { method: 'PATCH', path: '/idle-jobs/idle-a', body: { enabled: true } },
      { method: 'POST', path: '/idle-jobs/idle-a/run', body: undefined },
    ])
  })

  it('getIdleJobLog is GET /idle-jobs/{id}/log', async () => {
    const { bridge, api } = setup()
    bridge.answerWith(lines)
    expect(await api.getIdleJobLog('idle-a')).toEqual(lines)
    expect(bridge.calls).toEqual([{ method: 'GET', path: '/idle-jobs/idle-a/log', body: undefined }])
  })

  it('puts an id into the path in encoded form', async () => {
    const { bridge, api } = setup()
    bridge.answerWith(job)
    await api.setJobEnabled('a b/c', true)
    expect(bridge.calls[0]?.path).toBe('/jobs/a%20b%2Fc')
  })

  it('passes the backend error of a refused run through', async () => {
    const { bridge, api } = setup()
    const error = { code: 'conflict', message: 'Rain is getting ready for game mode.', reason: 'gameMode' }
    bridge.failWith(error)
    await expect(api.runJob('job-echo')).rejects.toEqual(error)
  })
})

describe('REST client: life', () => {
  const draft = {
    name: 'Gym',
    icon: { kind: 'lucide', value: 'dumbbell' },
    property: 'gym',
    cadence: 'weekly',
    times: 3,
  } as const

  it('getVaultStatus, rereadVault and listHabitSuggestions use /vault', async () => {
    const { bridge, api } = setup()
    await api.getVaultStatus()
    await api.rereadVault()
    await api.listHabitSuggestions()
    expect(bridge.calls).toEqual([
      { method: 'GET', path: '/vault', body: undefined },
      { method: 'POST', path: '/vault/reread', body: undefined },
      { method: 'GET', path: '/vault/properties', body: undefined },
    ])
  })

  it('listHabits is GET /habits', async () => {
    const { bridge, api } = setup()
    await api.listHabits()
    expect(bridge.calls).toEqual([{ method: 'GET', path: '/habits', body: undefined }])
  })

  it('saveHabit creates with POST and edits with PUT, by the id of the draft', async () => {
    const { bridge, api } = setup()
    await api.saveHabit(draft)
    await api.saveHabit({ ...draft, id: 'gym' })
    expect(bridge.calls).toEqual([
      { method: 'POST', path: '/habits', body: draft },
      { method: 'PUT', path: '/habits/gym', body: { ...draft, id: 'gym' } },
    ])
  })

  it('deleteHabit and getHabitStats put the id in the path, percent-encoded', async () => {
    const { bridge, api } = setup()
    await api.deleteHabit('gym')
    await api.getHabitStats('a b/c')
    expect(bridge.calls).toEqual([
      { method: 'DELETE', path: '/habits/gym', body: undefined },
      { method: 'GET', path: '/habits/a%20b%2Fc/stats', body: undefined },
    ])
  })

  it('listHabitDays sends the range, and the habit only when there is one', async () => {
    const { bridge, api } = setup()
    await api.listHabitDays('2026-01-01', '2026-11-22')
    await api.listHabitDays('2026-01-01', '2026-11-22', 'gym')
    expect(bridge.calls.map((call) => call.path)).toEqual([
      '/habit-days?from=2026-01-01&to=2026-11-22',
      '/habit-days?from=2026-01-01&to=2026-11-22&habit=gym',
    ])
  })

  it('setHabitDone is PUT /habits/:id/days/:date with { done }', async () => {
    const { bridge, api } = setup()
    await api.setHabitDone('gym', '2026-11-22', true)
    await api.setHabitDone('gym', '2026-11-22', false)
    expect(bridge.calls).toEqual([
      { method: 'PUT', path: '/habits/gym/days/2026-11-22', body: { done: true } },
      { method: 'PUT', path: '/habits/gym/days/2026-11-22', body: { done: false } },
    ])
  })

  it('listEvents and openDailyNote use the calendar and daily note routes', async () => {
    const { bridge, api } = setup()
    await api.listEvents('2026-11-01', '2026-11-30')
    await api.openDailyNote('2026-11-22')
    expect(bridge.calls).toEqual([
      { method: 'GET', path: '/calendar/events?from=2026-11-01&to=2026-11-30', body: undefined },
      { method: 'POST', path: '/daily-notes/2026-11-22/open', body: undefined },
    ])
  })
})

describe('REST client: game mode', () => {
  const state = { phase: 'countdown', rev: 3 }

  it('asks GET /game-mode and raises the mark of the stream without delivering', async () => {
    const { bridge, api } = setup()
    bridge.answerWith(state)
    expect(await api.getGameMode()).toEqual(state)
    expect(bridge.calls).toEqual([{ method: 'GET', path: '/game-mode', body: undefined }])
  })

  it('starts with an empty body, or with the game', async () => {
    const { bridge, api } = setup()
    bridge.answerWith(state)
    await api.startGameMode()
    await api.startGameMode({ thenLaunchGameId: 'game-a' })
    expect(bridge.calls.map((call) => call.body)).toEqual([{}, { thenLaunchGameId: 'game-a' }])
    expect(bridge.calls[0]).toMatchObject({ method: 'POST', path: '/game-mode' })
  })

  it('sends the choice and the recap mark to their routes', async () => {
    const { bridge, api } = setup()
    bridge.answerWith(state)
    await api.resolveGameMode('stopAnyway')
    await api.markGameModeRecapSeen()
    expect(bridge.calls).toEqual([
      { method: 'POST', path: '/game-mode/choice', body: { choice: 'stopAnyway' } },
      { method: 'POST', path: '/game-mode/recap/seen', body: undefined },
    ])
  })

  it('gives the returned state to the stream before the call resolves', async () => {
    const bridge = createFakeBridge()
    const delivered: unknown[] = []
    const api = createRestApi(bridge, {
      subscribe: () => () => {},
      deliver: (_topic, payload) => (delivered.push(payload), payload),
      note: (_topic, payload) => payload,
    })
    bridge.answerWith(state)
    await api.resolveGameMode('wait')
    expect(delivered).toEqual([state])
  })
})
