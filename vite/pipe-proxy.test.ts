import { readFileSync } from 'node:fs'
import { describe, expect, it } from 'vitest'
import { pipeHeaders, pipeNameFor, refusalFor, userSlug, type GuardInput } from './pipe-proxy.ts'

const PORT = 1420

// The same list is read by the Rust test of rain_types::env::user_slug, so the two
// implementations of the pipe-name rule cannot drift apart (foundation 2.7).
const SLUG_CASES = JSON.parse(
  readFileSync(new URL('../crates/rain-types/user-slug-cases.json', import.meta.url), 'utf8'),
) as { cases: { name: string; slug: string }[] }

describe('pipe name', () => {
  it.each(SLUG_CASES.cases)('user slug of $name', ({ name, slug }) => {
    expect(userSlug(name)).toBe(slug)
  })

  it('is rain-dev-<user> in the dev server, which is always a sandbox', () => {
    expect(pipeNameFor({ USERNAME: 'Ada Lovelace' })).toBe('rain-dev-ada-lovelace')
  })

  it('falls back to "user" when there is no usable user name', () => {
    expect(pipeNameFor({})).toBe('rain-dev-user')
    expect(pipeNameFor({ USERNAME: '' })).toBe('rain-dev-user')
  })

  it('RAIN_PIPE wins when it is valid', () => {
    expect(pipeNameFor({ RAIN_PIPE: 'rain-test-1.a_B', USERNAME: 'Ada' })).toBe('rain-test-1.a_B')
  })

  it('rejects a RAIN_PIPE that cannot be a pipe name, like the backend does', () => {
    for (const bad of ['', 'has space', 'a\\b', 'a/b', 'name:colon', 'é', 'x'.repeat(65)]) {
      expect(() => pipeNameFor({ RAIN_PIPE: bad })).toThrow('RAIN_PIPE')
    }
  })
})

describe('origin guard', () => {
  /** A request the dev page itself would make. */
  const good: GuardInput = {
    method: 'GET',
    url: '/api/v1/settings',
    host: 'localhost:1420',
    origin: undefined,
    secFetchSite: 'same-origin',
  }
  const refusal = (changes: Partial<GuardInput>) => refusalFor({ ...good, ...changes }, PORT)

  it('lets the dev page through, on either host name', () => {
    expect(refusal({})).toBeUndefined()
    expect(refusal({ host: '127.0.0.1:1420', origin: 'http://127.0.0.1:1420' })).toBeUndefined()
    expect(refusal({ origin: 'http://localhost:1420' })).toBeUndefined()
  })

  it('accepts a request without Origin and Sec-Fetch-Site (curl, "none")', () => {
    expect(refusal({ secFetchSite: undefined })).toBeUndefined()
    expect(refusal({ secFetchSite: 'none' })).toBeUndefined()
  })

  it('refuses a foreign Host (DNS rebinding)', () => {
    expect(refusal({ host: 'evil.example:1420' })).toBe('host')
    expect(refusal({ host: 'localhost:9999' })).toBe('host')
    expect(refusal({ host: 'localhost' })).toBe('host')
    expect(refusal({ host: undefined })).toBe('host')
  })

  it('refuses a foreign Origin', () => {
    expect(refusal({ origin: 'http://evil.example' })).toBe('origin')
    expect(refusal({ origin: 'https://localhost:1420' })).toBe('origin')
    expect(refusal({ origin: 'http://localhost:1421' })).toBe('origin')
    expect(refusal({ origin: 'null' })).toBe('origin')
  })

  it('refuses a cross-site or same-site request', () => {
    expect(refusal({ secFetchSite: 'cross-site' })).toBe('site')
    expect(refusal({ secFetchSite: 'same-site' })).toBe('site')
  })

  it('refuses /internal in every spelling', () => {
    for (const url of [
      '/api/v1/internal/shutdown',
      '/api/v1/internal',
      '/api/v1/internal?x=1',
      '/api/v1//internal/hook',
      '/api/v1/INTERNAL/hook',
      '/api/v1/%69nternal/hook',
      '/api/v1/%2e%2e/internal',
      '/api/v1/settings/../internal/shell',
      '/api/v1/%zz',
    ]) {
      expect(refusal({ url }), url).toBe('internal')
    }
  })

  it('does not refuse a path that only contains the word internal', () => {
    expect(refusal({ url: '/api/v1/settings?note=internal' })).toBeUndefined()
    expect(refusal({ url: '/api/v1/pools/internal-tools' })).toBeUndefined()
  })

  it('needs X-Rain-Dev: 1 on every method but GET', () => {
    for (const method of ['POST', 'PUT', 'PATCH', 'DELETE', 'OPTIONS', 'HEAD']) {
      expect(refusal({ method }), method).toBe('devHeader')
      expect(refusal({ method, devHeader: '0' }), method).toBe('devHeader')
      expect(refusal({ method, devHeader: '1' }), method).toBeUndefined()
    }
  })
})

describe('headers sent down the pipe', () => {
  it('forwards the body headers and If-None-Match, so a cached image costs a 304', () => {
    expect(
      pipeHeaders({ 'content-type': 'application/json', 'content-length': '2', 'if-none-match': '"app-icon-a-1"' }),
    ).toEqual({
      host: 'rain',
      'content-type': 'application/json',
      'content-length': '2',
      'if-none-match': '"app-icon-a-1"',
    })
  })

  it('drops everything that identifies the page: Origin, Cookie, X-Rain-Dev, Referer', () => {
    const headers = pipeHeaders({
      origin: 'http://localhost:1420',
      cookie: 'a=b',
      'x-rain-dev': '1',
      referer: 'http://localhost:1420/',
      'user-agent': 'x',
    })
    expect(headers).toEqual({ host: 'rain' })
  })
})
