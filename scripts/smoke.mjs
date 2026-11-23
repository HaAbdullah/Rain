// "The app starts" check (foundation section 12.5).   npm run smoke
//
// Builds the backend and the shell, starts the backend on a unique pipe in a temporary folder,
// reads `hello` and `resync` from its stream, starts the shell hidden and waits until the
// backend reports the shell's facts, reads the default settings, then ends everything and fails
// if anything of ours is still running or the pipe is still there. No window is shown.
import { spawn, spawnSync } from 'node:child_process'
import { mkdtempSync, rmSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { API, isAlive, pipeExists, pipeRequest, readFirstFrames, sleep, waitForPing } from './lib/pipe.mjs'

const STOP_WITHIN_MS = 5000
const SHELL_REPORTS_WITHIN_MS = 20000
const backendExe = join('target', 'debug', 'rain-backend.exe')
const shellExe = join('target', 'debug', 'rain.exe')

const tempDir = mkdtempSync(join(tmpdir(), 'rain-smoke-'))
const pipeName = `rain-smoke-${process.pid}-${Date.now().toString(36)}`
const env = {
  ...process.env,
  RAIN_PIPE: pipeName,
  RAIN_DATA_DIR: join(tempDir, 'data'),
  RAIN_LOCAL_DIR: join(tempDir, 'local'),
}

// What `GET /settings` answers for the fields that do not depend on this PC, with an empty data
// folder. `hotkeyAvailable` follows what the shell reports (checked separately). `dataDir` is
// this run's folder. The detected fields (vault, daily notes, Steam, Epic) answer what the
// machine has, so only their type is checked.
const DEFAULT_SETTINGS = {
  hotkey: 'Ctrl+Backquote',
  hideOnBlur: true,
  resetAfterSeconds: 60,
  startWithWindows: false,
  dataDir: join(tempDir, 'data'),
  projectRoots: [],
  gameFolders: [],
  steamEnabled: true,
  epicEnabled: true,
  calendarConnected: false,
  monitorClaude: false,
  monitorClaudeState: 'off',
  gameModeCountdownSeconds: 10,
}
const DETECTED_TEXT_FIELDS = ['vaultPath', 'dailyNoteFormat', 'dailyNoteFolder', 'steamPath', 'epicPath']

let backend = null // the processes we started, until we know they have ended
let shell = null

function fail(message) {
  throw new Error(message)
}

function step(text) {
  console.log(`smoke: ${text}`)
}

async function getJson(path) {
  const { status, body } = await pipeRequest(pipeName, `${API}${path}`)
  if (status !== 200) fail(`GET ${path} answered ${status}: ${body}`)
  return JSON.parse(body)
}

/** Ends a process and everything it started (the shell has WebView2 helper processes). */
function killTree(pid) {
  spawnSync('taskkill', ['/PID', String(pid), '/T', '/F'], { stdio: 'ignore' })
}

async function waitForShellFacts() {
  const deadline = Date.now() + SHELL_REPORTS_WITHIN_MS
  while (Date.now() < deadline) {
    if (!shell) {
      fail('the shell ended at once. Is another Rain running? A second start only wakes the first.')
    }
    const health = await getJson('/health')
    if (health.shell.reported) return health
    await sleep(200)
  }
  return fail(`the shell did not report its facts within ${SHELL_REPORTS_WITHIN_MS / 1000} s`)
}

async function run() {
  step('1. build the backend and the shell (debug)')
  const build = spawnSync('cargo', ['build', '-p', 'rain-backend', '-p', 'rain'], { stdio: 'inherit' })
  if (build.status !== 0) fail('cargo build failed')

  step(`2. start ${backendExe} on pipe ${pipeName}`)
  backend = spawn(backendExe, [], { env, stdio: 'ignore', windowsHide: true })
  backend.on('exit', () => (backend = null))
  const ping = await waitForPing(pipeName)
  if (!ping) fail('the backend did not answer /ping within 10 s')
  if (ping.app !== 'rain-backend') fail(`unexpected ping answer: ${JSON.stringify(ping)}`)
  if (ping.sandbox !== true) fail('a smoke run must be in sandbox mode')
  step(`   ping ok: version ${ping.version}, pid ${ping.pid}, sandbox ${ping.sandbox}`)
  const [hello, resync] = await readFirstFrames(pipeName, 2)
  if (hello.topic !== 'hello' || hello.payload.startId !== ping.startId) {
    fail(`first frame is not the hello of this backend: ${JSON.stringify(hello)}`)
  }
  if (resync.topic !== 'resync' || resync.payload.reason !== 'connect') {
    fail(`second frame is not resync/connect: ${JSON.stringify(resync)}`)
  }
  step(`   stream: hello (startId ${hello.payload.startId}) then resync (${resync.payload.reason})`)

  step('3. start the shell hidden and wait for its facts')
  shell = spawn(shellExe, ['--hidden'], { env, stdio: 'ignore', windowsHide: true })
  const shellPid = shell.pid
  shell.on('exit', () => (shell = null))
  const health = await waitForShellFacts()
  step(`   health: shell reported, hotkeyRegistered ${health.shell.hotkeyRegistered}`)

  step('4. read the settings through the pipe')
  const { hotkeyAvailable, ...settings } = await getJson('/settings')
  for (const field of DETECTED_TEXT_FIELDS) {
    if (typeof settings[field] !== 'string') fail(`${field} is not a string: ${JSON.stringify(settings[field])}`)
    delete settings[field]
  }
  if (JSON.stringify(settings) !== JSON.stringify(DEFAULT_SETTINGS)) {
    fail(`settings differ from the defaults: ${JSON.stringify(settings)}`)
  }
  if (hotkeyAvailable !== health.shell.hotkeyRegistered) {
    fail(`hotkeyAvailable (${hotkeyAvailable}) does not follow what the shell reported`)
  }
  step('   the defaults, and hotkeyAvailable follows the shell')

  step('5. end the shell, stop the backend, check that nothing is left')
  killTree(shellPid)
  while (shell) await sleep(50) // the 'exit' event clears it
  const stopped = Date.now()
  const stop = spawnSync(backendExe, ['--stop'], { env, stdio: 'inherit' })
  if (stop.status !== 0) fail(`--stop exited with ${stop.status}`)
  while (backend && Date.now() - stopped < STOP_WITHIN_MS) await sleep(50)
  if (backend) fail(`the backend was still running ${STOP_WITHIN_MS} ms after --stop`)
  step(`   backend ended ${Date.now() - stopped} ms after --stop`)
  if (await pipeExists(pipeName)) fail('the pipe still exists after the backend ended')
  if (isAlive(ping.pid)) fail(`backend process ${ping.pid} is still running`)
  if (isAlive(shellPid)) fail(`shell process ${shellPid} is still running`)
  step('smoke check passed')
}

let exitCode = 0
try {
  await run()
} catch (error) {
  console.error(`smoke check FAILED: ${error.message}`)
  exitCode = 1
} finally {
  // Whatever happened, do not leave our processes behind.
  if (shell) {
    killTree(shell.pid)
    console.error('smoke: the shell had to be killed')
    exitCode = 1
  }
  if (backend) {
    backend.kill()
    console.error('smoke: the backend had to be killed')
    exitCode = 1
  }
  try {
    rmSync(tempDir, { recursive: true, force: true })
  } catch {
    // a log file may still be locked for a moment; the folder is in the temp folder anyway
  }
}
process.exit(exitCode)
