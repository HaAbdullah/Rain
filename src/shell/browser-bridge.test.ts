import { afterEach, describe, expect, it, vi } from 'vitest'
import { createBrowserBridge, parseFrame, reconnectDelayMs } from './browser-bridge'

function respondWith(body: string, status = 200) {
  const fetchMock = vi.fn().mockResolvedValue(new Response(status === 204 ? null : body, { status }))
  vi.stubGlobal('fetch', fetchMock)
  return fetchMock
}

afterEach(() => vi.unstubAllGlobals())

describe('browser bridge: request', () => {
  const bridge = createBrowserBridge()

  it('GETs /api/v1 plus the path and parses the JSON', async () => {
    const fetchMock = respondWith('{"hotkey":"Ctrl+Backquote"}')

    expect(await bridge.request({ method: 'GET', path: '/settings' })).toEqual({ hotkey: 'Ctrl+Backquote' })

    const [url, init] = fetchMock.mock.calls[0] as [string, RequestInit]
    expect(url).toBe('/api/v1/settings')
    expect(init.method).toBe('GET')
    expect(init.headers).toEqual({})
    expect(init.body).toBeUndefined()
  })

  it('sends a body as JSON, and marks every call that is not a GET for the dev proxy', async () => {
    const fetchMock = respondWith('{}')

    await bridge.request({ method: 'PATCH', path: '/settings', body: { hideOnBlur: false } })

    const [, init] = fetchMock.mock.calls[0] as [string, RequestInit]
    expect(init.body).toBe('{"hideOnBlur":false}')
    expect(init.headers).toEqual({ 'X-Rain-Dev': '1', 'Content-Type': 'application/json' })
  })

  it('resolves undefined for 204', async () => {
    respondWith('', 204)
    expect(await bridge.request({ method: 'DELETE', path: '/pools/p1' })).toBeUndefined()
  })

  it('rejects with the error body the backend sent', async () => {
    const error = { code: 'conflict', message: 'Busy', source: 'jobs', reason: 'busy' }
    respondWith(JSON.stringify(error), 409)

    await expect(bridge.request({ method: 'POST', path: '/jobs/j1/runs' })).rejects.toEqual(error)
  })

  it('a 503 that is not an error body is unavailable with reason badResponse', async () => {
    respondWith('<html>Bad gateway</html>', 503)

    await expect(bridge.request({ method: 'GET', path: '/settings' })).rejects.toMatchObject({
      code: 'unavailable',
      source: 'backend',
      reason: 'badResponse',
    })
  })

  it('maps the status of an answer without an error body: 404 notFound, 500 failed', async () => {
    respondWith('{"unexpected":true}', 404)
    await expect(bridge.request({ method: 'GET', path: '/x' })).rejects.toMatchObject({ code: 'notFound' })

    respondWith('{"unexpected":true}', 500)
    await expect(bridge.request({ method: 'GET', path: '/x' })).rejects.toMatchObject({ code: 'failed' })
  })

  it('a 200 that is not JSON is failed with reason badResponse', async () => {
    respondWith('not json')

    await expect(bridge.request({ method: 'GET', path: '/settings' })).rejects.toMatchObject({
      code: 'failed',
      reason: 'badResponse',
    })
  })

  it('a failed fetch is unavailable with reason transport', async () => {
    vi.stubGlobal('fetch', vi.fn().mockRejectedValue(new TypeError('Failed to fetch')))

    await expect(bridge.request({ method: 'GET', path: '/settings' })).rejects.toMatchObject({
      code: 'unavailable',
      source: 'backend',
      reason: 'transport',
    })
  })

  it('a timeout is unavailable with reason timeout', async () => {
    vi.stubGlobal('fetch', vi.fn().mockRejectedValue(new DOMException('timed out', 'TimeoutError')))

    await expect(bridge.request({ method: 'GET', path: '/settings' })).rejects.toMatchObject({
      code: 'unavailable',
      reason: 'timeout',
    })
  })
})

describe('browser bridge: media and reconnect', () => {
  it('builds media addresses under the API base', () => {
    expect(createBrowserBridge().mediaUrl('media/app-icon/app-vscode?v=1')).toBe('/api/v1/media/app-icon/app-vscode?v=1')
  })

  it('waits 0.5 s, 1 s, 2 s and then 5 s before each reconnect', () => {
    expect([0, 1, 2, 3, 10].map(reconnectDelayMs)).toEqual([500, 1000, 2000, 5000, 5000])
  })
})

describe('browser bridge: frames', () => {
  it('reads a frame with a topic and a sequence number', () => {
    expect(parseFrame('{"topic":"settings","seq":3,"payload":{"a":1}}')).toEqual({
      topic: 'settings',
      seq: 3,
      payload: { a: 1 },
    })
  })

  it('drops JSON that is not a frame, and text that is not JSON', () => {
    for (const data of ['5', 'null', '{}', '{"topic":1,"seq":1}', '{"topic":"x"}', '[1]', 'not json']) {
      expect(parseFrame(data), data).toBeUndefined()
    }
  })
})
