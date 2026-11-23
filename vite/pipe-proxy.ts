// Development only (foundation section 7): lets a plain browser tab at http://localhost:1420
// reach the Rain backend. A browser cannot open a Windows named pipe, but this dev server
// (Node) can, so it forwards every `/api/v1/` request, and the live stream, to the pipe.
//
// The pipe is protected by Windows, but this proxy is a door a web page could try, so every
// request passes a guard first: only this dev page may call, and `/internal/` is never reachable.
// The plugin has `apply: 'serve'`, so `vite build` never contains any of it.
import http from 'node:http'
import type { Plugin } from 'vite'
import type { ApiError } from '../src/bindings/rain-types.ts'

const API_PREFIX = '/api/v1/'
const PIPE_PATH_PREFIX = '\\\\.\\pipe\\'

/** A call that is not a plain GET must carry this header; a foreign page cannot add it without a pre-flight. */
const DEV_HEADER = 'x-rain-dev'

// ---- the pipe name: the same rule as `rain_types::env` (foundation 2.7 and 3.1) ----

const USER_SLUG_MAX = 32
const FALLBACK_USER = 'user'
const PIPE_NAME_PATTERN = /^[A-Za-z0-9._-]{1,64}$/

/**
 * Lower-cases a Windows user name and keeps only a-z and 0-9; every other character becomes
 * one `-`. Cut to 32 characters. `Array.from` walks code points, like Rust's `chars()`, so a
 * character outside the BMP (an emoji) gives one dash, not two.
 */
export function userSlug(userName: string): string {
  return Array.from(userName.toLowerCase(), (char) => (/^[a-z0-9]$/.test(char) ? char : '-'))
    .slice(0, USER_SLUG_MAX)
    .join('')
}

/** `RAIN_PIPE` if set, else `rain-dev-<user>`: the dev server always talks to the sandbox backend. */
export function pipeNameFor(env: { RAIN_PIPE?: string; USERNAME?: string }): string {
  if (env.RAIN_PIPE !== undefined) {
    if (!PIPE_NAME_PATTERN.test(env.RAIN_PIPE)) {
      throw new Error(`RAIN_PIPE must be 1 to 64 characters from A-Z a-z 0-9 . _ - (got "${env.RAIN_PIPE}")`)
    }
    return env.RAIN_PIPE
  }
  return `rain-dev-${userSlug(env.USERNAME ?? '') || FALLBACK_USER}`
}

// ---- the guard ----

/** What the guard looks at. */
export interface GuardInput {
  method: string
  /** The request target as the browser sent it: path and query. */
  url: string
  host?: string
  origin?: string
  secFetchSite?: string
  devHeader?: string
}

function decodeOrNull(text: string): string | null {
  try {
    return decodeURIComponent(text)
  } catch {
    return null
  }
}

/**
 * True for `/api/v1/internal` and everything below it, also when it is written with extra
 * slashes or percent codes, and for any path that climbs with `..`.
 */
function reachesInternal(url: string): boolean {
  const rawPath = url.split('?')[0] ?? ''
  const decoded = decodeOrNull(rawPath)
  if (decoded === null) return true // cannot be read, so cannot be trusted
  return [rawPath, decoded].some(
    (path) => /^\/api\/v1\/+internal(\/|$)/i.test(path) || /(^|[/\\])\.\.([/\\]|$)/.test(path),
  )
}

/**
 * Returns undefined when the request may pass, or the name of the check it failed.
 * The checks are those of foundation section 7, step 1.
 */
export function refusalFor(input: GuardInput, port: number): string | undefined {
  const hosts = [`localhost:${port}`, `127.0.0.1:${port}`]
  if (input.host === undefined || !hosts.includes(input.host)) return 'host'
  if (input.origin !== undefined && !hosts.some((host) => input.origin === `http://${host}`)) return 'origin'
  if (input.secFetchSite !== undefined && !['same-origin', 'none'].includes(input.secFetchSite)) return 'site'
  if (reachesInternal(input.url)) return 'internal'
  if (input.method !== 'GET' && input.devHeader !== '1') return 'devHeader'
  return undefined
}

// ---- answers the proxy writes itself ----

function sendError(res: http.ServerResponse, status: number, error: ApiError) {
  res.writeHead(status, { 'Content-Type': 'application/json; charset=utf-8', 'Cache-Control': 'no-store' })
  res.end(JSON.stringify(error))
}

const FORBIDDEN: ApiError = {
  code: 'failed',
  message: 'The Rain dev proxy refused this request',
  source: 'backend',
  reason: 'forbidden',
}

/** The `unavailable` error for a pipe that cannot be reached (foundation 6.6), by Node's error code. */
function unreachable(error: NodeJS.ErrnoException): ApiError {
  if (error.code === 'EACCES' || error.code === 'EPERM') {
    return {
      code: 'unavailable',
      message: 'The Rain backend belongs to another user',
      source: 'backend',
      reason: 'pipeDenied',
    }
  }
  if (error.code === 'ENOENT' || error.code === 'ECONNREFUSED') {
    return {
      code: 'unavailable',
      message: 'Rain backend is not running. Start it with: npm run backend:start',
      source: 'backend',
      reason: 'backendDown',
    }
  }
  return { code: 'unavailable', message: 'Could not reach the Rain backend', source: 'backend', reason: 'transport' }
}

// ---- the proxy ----

function header(req: http.IncomingMessage, name: string): string | undefined {
  const value = req.headers[name]
  return Array.isArray(value) ? value[0] : value
}

/**
 * The only headers that go down the pipe: no Origin, no Cookie, nothing of the page's identity.
 * `if-none-match` is what makes an image cached by the browser cost a `304` and no bytes
 * (foundation 4.7). `host` is the placeholder authority the backend expects.
 */
const FORWARDED_HEADERS = ['content-type', 'content-length', 'if-none-match']

export function pipeHeaders(incoming: http.IncomingHttpHeaders): http.OutgoingHttpHeaders {
  const headers: http.OutgoingHttpHeaders = { host: 'rain' }
  for (const name of FORWARDED_HEADERS) {
    const value = incoming[name]
    if (value !== undefined) headers[name] = value
  }
  return headers
}

/** A write error that only says the backend already closed the pipe after it answered. */
function isClosedPipe(error: NodeJS.ErrnoException): boolean {
  return error.code === 'EPIPE' || error.code === 'ECONNRESET'
}

/** Reads a whole request body; a request without one gives an empty buffer. */
function readBody(req: http.IncomingMessage): Promise<Buffer> {
  return new Promise((resolve, reject) => {
    const chunks: Buffer[] = []
    req.on('data', (chunk: Buffer) => chunks.push(chunk))
    req.on('end', () => resolve(Buffer.concat(chunks)))
    req.on('error', reject)
  })
}

/**
 * Sends one request down the pipe and streams the answer back unchanged, so SSE and images work.
 * The request body (small JSON) is read first and written in one piece, so the backend never
 * answers before the body is complete and the pipe never sees a half-written request.
 */
async function forward(req: http.IncomingMessage, res: http.ServerResponse, pipeName: string) {
  let body: Buffer
  try {
    body = await readBody(req)
  } catch {
    res.destroy()
    return
  }
  let answered = false
  const upstream = http.request(
    { socketPath: PIPE_PATH_PREFIX + pipeName, method: req.method, path: req.url, headers: pipeHeaders(req.headers), agent: false },
    (answer) => {
      answered = true
      res.writeHead(answer.statusCode ?? 502, answer.headers)
      res.flushHeaders()
      answer.pipe(res)
      answer.on('error', () => res.destroy())
    },
  )
  upstream.on('error', (error: NodeJS.ErrnoException) => {
    // Once the answer has started, a closed pipe is the normal end of the exchange.
    if (answered && isClosedPipe(error)) return
    if (res.headersSent) res.destroy() // the stream broke half way: the browser's EventSource reconnects
    else sendError(res, 503, unreachable(error))
  })
  // The browser went away (a closed tab, a stopped stream): do not keep the pipe open for nobody.
  res.on('close', () => upstream.destroy())
  upstream.end(body)
}

/** The dev server port is read from the resolved Vite config, so it is not written down twice. */
export function rainPipeProxy(): Plugin {
  return {
    name: 'rain-pipe-proxy',
    apply: 'serve',
    configureServer(server) {
      const pipeName = pipeNameFor(process.env)
      const port = server.config.server.port
      server.middlewares.use((req, res, next) => {
        const url = req.url ?? ''
        if (!url.startsWith(API_PREFIX)) return next()

        const refusal = refusalFor(
          {
            method: req.method ?? 'GET',
            url,
            host: header(req, 'host'),
            origin: header(req, 'origin'),
            secFetchSite: header(req, 'sec-fetch-site'),
            devHeader: header(req, DEV_HEADER),
          },
          port,
        )
        if (refusal) sendError(res, 403, FORBIDDEN)
        else void forward(req, res, pipeName)
      })
    },
  }
}
