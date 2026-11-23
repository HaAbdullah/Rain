// The idle measurement (foundation stage 2, checks S10 and the 14.2 acceptance).
//
//   npm run idle                  the debug backend, 60 seconds
//   npm run idle -- --release     the release backend (build it first: cargo build --release -p rain-backend)
//   npm run idle -- --seconds 120
//
// Starts the backend on a unique pipe in temporary folders (a sandbox run, no window), lets it
// settle for 3 seconds, then measures the working set and the CPU time it gained over the
// measured time. Fails when it is over the budget: 30 MB working set, 0.1 s CPU gained.
// Stops the backend and removes the folders at the end, also after a failure.
import { spawn, spawnSync } from 'node:child_process'
import { mkdtempSync, rmSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { sleep, waitForPing } from './lib/pipe.mjs'

const MAX_WORKING_SET_BYTES = 30 * 1024 * 1024
const MAX_CPU_GAINED_MS = 100
const SETTLE_MS = 3000

const args = process.argv.slice(2)
const release = args.includes('--release')
const secondsAt = args.indexOf('--seconds')
const seconds = secondsAt === -1 ? 60 : Number(args[secondsAt + 1])
const exe = join('target', release ? 'release' : 'debug', 'rain-backend.exe')

const tempDir = mkdtempSync(join(tmpdir(), 'rain-idle-'))
const pipeName = `rain-idle-${process.pid}`
const env = {
  ...process.env,
  RAIN_PIPE: pipeName,
  RAIN_DATA_DIR: join(tempDir, 'data'),
  RAIN_LOCAL_DIR: join(tempDir, 'local'),
}

/** Working set (bytes) and total CPU time (ms) of a process, read with PowerShell's Get-Process. */
function measure(pid) {
  const script = `$p = Get-Process -Id ${pid}; "$($p.WorkingSet64) $($p.TotalProcessorTime.TotalMilliseconds) $($p.Threads.Count)"`
  const result = spawnSync('powershell', ['-NoProfile', '-Command', script], { encoding: 'utf8' })
  const [workingSet, cpuMs, threads] = result.stdout.trim().split(' ').map(Number)
  if (!Number.isFinite(workingSet)) throw new Error(`could not read the process: ${result.stderr}`)
  return { workingSet, cpuMs, threads }
}

let exitCode = 0
const child = spawn(exe, [], { env, stdio: 'ignore', windowsHide: true })
try {
  const ping = await waitForPing(pipeName)
  if (!ping) throw new Error(`${exe} did not answer /ping within 10 s (is it built?)`)
  console.log(`idle: ${exe} (pid ${ping.pid}), settling ${SETTLE_MS / 1000} s, then measuring ${seconds} s`)
  await sleep(SETTLE_MS)
  const before = measure(ping.pid)
  await sleep(seconds * 1000)
  const after = measure(ping.pid)

  const cpuGained = after.cpuMs - before.cpuMs
  const megabytes = (after.workingSet / 1024 / 1024).toFixed(1)
  console.log(`idle: working set ${megabytes} MB, CPU gained ${cpuGained.toFixed(0)} ms, ${after.threads} threads`)
  if (after.workingSet > MAX_WORKING_SET_BYTES) throw new Error('over the 30 MB working set budget')
  if (cpuGained > MAX_CPU_GAINED_MS) throw new Error('over the 0.1 s CPU budget')
  console.log('idle: within the budget')
} catch (error) {
  console.error(`idle check FAILED: ${error.message}`)
  exitCode = 1
} finally {
  spawnSync(exe, ['--stop', '--force'], { env, stdio: 'ignore' })
  await sleep(300)
  child.kill()
  rmSync(tempDir, { recursive: true, force: true })
}
process.exit(exitCode)
