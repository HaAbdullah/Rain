// The development command (foundation section 2.8). `npm run dev` runs this, and it is also the
// `beforeDevCommand` of Tauri, so `npm run tauri dev` and a plain browser use the same backend.
//
//   npm run dev                       the backend on the normal sandbox folders, then vite
//   npm run dev:fixture               the backend on the `typical` fixture folder, then vite
//   node scripts/dev.mjs --fixture empty|broken
//
// What it does: stop a running backend, build it, start it detached, watch the crates and do
// the same again when a Rust file changes (also regenerating the TypeScript types when
// `rain-types` changed), and run vite. A build error is printed and the old backend stays
// stopped, so the screens show their error rows until the next successful build.
// The backend is detached on purpose: it keeps running when this command ends.
import { spawn } from 'node:child_process'
import { watch } from 'node:fs'
import { join } from 'node:path'
import {
  REPO,
  applyFixture,
  backendEnv,
  clearFixture,
  fixtureFromArgs,
  restartBackend,
  stopBackend,
  buildBackend,
  startBackend,
} from './backend.mjs'

// Long enough for an editor that writes a file in two steps, short enough not to be felt.
const REBUILD_DEBOUNCE_MS = 250

/** True for a change that needs a new backend: a source file or a Cargo.toml of a crate. */
function affectsBackend(changedPath) {
  const parts = changedPath.split(/[\\/]/)
  return parts.includes('src') || parts.at(-1) === 'Cargo.toml'
}

function changesTypes(changedPath) {
  return changedPath.split(/[\\/]/)[0] === 'rain-types'
}

function regenerateTypes() {
  console.log('dev: rain-types changed, regenerating src/bindings/rain-types.ts')
  return new Promise((resolve) => {
    // `shell: true` because npm is a .cmd file on Windows and cannot be started directly.
    spawn('npm run types', { stdio: 'inherit', shell: true }).on('exit', (code) => resolve(code === 0))
  })
}

/** Runs one rebuild cycle at a time; a change that arrives meanwhile starts one more afterwards. */
function startWatching(env) {
  let timer = null
  let typesChanged = false
  let busy = false
  let again = false

  async function cycle() {
    if (busy) {
      again = true
      return
    }
    busy = true
    try {
      do {
        again = false
        console.log('dev: Rust files changed, rebuilding the backend')
        stopBackend(env)
        if (typesChanged) {
          typesChanged = false
          await regenerateTypes()
        }
        if (buildBackend()) await startBackend(env)
        else console.error('dev: the build failed; the backend stays stopped until the next change')
      } while (again)
    } catch (error) {
      console.error(`dev: ${error.message}`)
    } finally {
      busy = false
    }
  }

  watch(join(REPO, 'crates'), { recursive: true }, (_event, changedPath) => {
    if (!changedPath || !affectsBackend(changedPath)) return
    if (changesTypes(changedPath)) typesChanged = true
    clearTimeout(timer)
    timer = setTimeout(cycle, REBUILD_DEBOUNCE_MS)
  })
}

function runVite() {
  const viteBin = join(REPO, 'node_modules', 'vite', 'bin', 'vite.js')
  const vite = spawn(process.execPath, [viteBin], { stdio: 'inherit', cwd: REPO })
  vite.on('exit', (code) => process.exit(code ?? 0))
  // Ctrl+C reaches vite too (same console); closing this script by other means must end it.
  process.on('exit', () => vite.kill())
}

const fixture = fixtureFromArgs(process.argv.slice(2))
if (fixture) applyFixture(fixture)
else clearFixture()

const env = backendEnv()
try {
  await restartBackend(env)
} catch (error) {
  console.error(`dev: ${error.message}`)
}
startWatching(env)
runVite()
