// Starts, stops and restarts the sandbox backend for development (foundation sections 2.8, 12.3).
//
//   npm run backend:start      stop it, build it, start it detached      (same as restart)
//   npm run backend:stop       ask it to stop
//   npm run backend:restart    stop it, build it, start it detached
//   node scripts/backend.mjs start --fixture typical|empty|broken
//   node scripts/backend.mjs fault apps normal|empty|fail   the launcher's app list answers normally, with nothing, or with an error
//
// Windows refuses to overwrite a running `rain-backend.exe`, so every start stops first. The
// stop command is the same program, which Windows allows to run a second time.
//
// `--fixture` copies a data folder from the tests to `.rain-dev/data` (replacing what is there)
// and starts the backend on it. The choice is remembered, so a later `backend:restart` without
// the flag keeps using that folder. `dev.mjs` without the flag goes back to the normal sandbox.
import { spawn, spawnSync } from 'node:child_process'
import { cpSync, existsSync, mkdirSync, readFileSync, rmSync, writeFileSync } from 'node:fs'
import { dirname, join } from 'node:path'
import { fileURLToPath, pathToFileURL } from 'node:url'
import { pipeNameFor } from '../vite/pipe-proxy.ts'
import { API, pipeExists, pipeRequest, sleep, waitForPing } from './lib/pipe.mjs'

export const REPO = join(dirname(fileURLToPath(import.meta.url)), '..')
export const BACKEND_EXE = join(REPO, 'target', 'debug', 'rain-backend.exe')

const FIXTURE_NAMES = ['typical', 'empty', 'broken']
const FIXTURE_SOURCE = join(REPO, 'crates', 'rain-backend', 'tests', 'fixtures')
const SANDBOX_DIR = join(REPO, '.rain-dev') // ignored by git and by the Vite watcher
const FIXTURE_DATA_DIR = join(SANDBOX_DIR, 'data')
const FIXTURE_MARKER = join(SANDBOX_DIR, 'fixture') // holds the name of the fixture in use
const STOP_WAIT_MS = 6000

export function fixtureFromArgs(args) {
  const at = args.lastIndexOf('--fixture')
  if (at === -1) return undefined
  const name = args[at + 1]
  if (!FIXTURE_NAMES.includes(name)) {
    throw new Error(`--fixture needs one of: ${FIXTURE_NAMES.join(', ')}`)
  }
  return name
}

/** Replaces `.rain-dev/data` with the fixture's files (an empty folder for `empty`). */
export function applyFixture(name) {
  rmSync(FIXTURE_DATA_DIR, { recursive: true, force: true })
  mkdirSync(SANDBOX_DIR, { recursive: true })
  const source = join(FIXTURE_SOURCE, name, 'data')
  if (existsSync(source)) cpSync(source, FIXTURE_DATA_DIR, { recursive: true })
  else mkdirSync(FIXTURE_DATA_DIR) // git keeps no empty folders, so `empty` has no source
  writeFileSync(FIXTURE_MARKER, name)
}

export function clearFixture() {
  rmSync(FIXTURE_MARKER, { force: true })
}

/** The environment of the backend: unchanged, or pointed at the fixture folder when one is in use. */
export function backendEnv() {
  return existsSync(FIXTURE_MARKER) ? { ...process.env, RAIN_DATA_DIR: FIXTURE_DATA_DIR } : { ...process.env }
}

/** Where this backend writes its log. */
export function logFolder(env) {
  if (env.RAIN_DATA_DIR) return `${env.RAIN_DATA_DIR}.local\\logs`
  return join(env.LOCALAPPDATA ?? '', 'Rain-dev', 'logs')
}

export function stopBackend(env) {
  if (!existsSync(BACKEND_EXE)) return true // never built, so nothing can be running
  return spawnSync(BACKEND_EXE, ['--stop'], { env, stdio: 'inherit' }).status === 0
}

/** Builds the backend. A build error is printed by cargo; the caller decides what follows. */
export function buildBackend() {
  console.log('backend: building (cargo build -p rain-backend)')
  return spawnSync('cargo', ['build', '-p', 'rain-backend'], { stdio: 'inherit' }).status === 0
}

/** Starts the backend detached from this process, so it lives on when the script ends. */
export async function startBackend(env) {
  const pipeName = pipeNameFor(env)
  let exitCode = null
  const child = spawn(BACKEND_EXE, [], { env, detached: true, stdio: 'ignore', windowsHide: true })
  child.on('exit', (code) => (exitCode = code ?? -1))
  child.unref()

  const answer = await waitForPing(pipeName)
  if (!answer) {
    const why = exitCode === null ? 'it did not answer within 10 s' : `it ended with code ${exitCode}`
    throw new Error(`the backend did not start: ${why}. Log: ${logFolder(env)}`)
  }
  console.log(`backend: running (pid ${answer.pid}, pipe ${pipeName}, sandbox ${answer.sandbox})`)
  console.log(`backend: log folder ${logFolder(env)}`)
}

/** Stop, build, start. Returns false when it stopped at a step that failed (already reported). */
export async function restartBackend(env = backendEnv()) {
  if (!stopBackend(env)) return false
  const pipeName = pipeNameFor(env)
  const deadline = Date.now() + STOP_WAIT_MS
  while ((await pipeExists(pipeName)) && Date.now() < deadline) await sleep(50)
  if (!buildBackend()) {
    console.error('backend: the build failed; the backend stays stopped')
    return false
  }
  await startBackend(env)
  return true
}

/**
 * The dev-only fault route of the launcher (section 01 11.3): the app list answers normally,
 * with nothing, or with an error, so that the empty and failing screens can be seen.
 */
async function fault([area, mode]) {
  if (area !== 'apps' || !['normal', 'empty', 'fail'].includes(mode)) {
    console.error('Usage: node scripts/backend.mjs fault apps normal|empty|fail')
    return false
  }
  const { status, body } = await pipeRequest(pipeNameFor(backendEnv()), `${API}/internal/test/apps-source`, {
    method: 'PUT',
    body: { mode },
  })
  if (status !== 204) {
    console.error(`backend: the fault route answered ${status} ${body}`)
    return false
  }
  console.log(`backend: the app source now answers "${mode}"`)
  return true
}

async function main(args) {
  const verb = args[0]
  if (verb === 'fault') return fault(args.slice(1))
  if (verb === 'stop') return stopBackend(backendEnv())
  if (verb === 'start' || verb === 'restart') {
    const fixture = fixtureFromArgs(args)
    if (fixture) applyFixture(fixture)
    return restartBackend()
  }
  console.error('Usage: node scripts/backend.mjs start|stop|restart [--fixture typical|empty|broken]')
  return false
}

// Run as a command (not when dev.mjs imports this file).
if (import.meta.url === pathToFileURL(process.argv[1]).href) {
  try {
    process.exit((await main(process.argv.slice(2))) ? 0 : 1)
  } catch (error) {
    console.error(`backend: ${error.message}`)
    process.exit(1)
  }
}
