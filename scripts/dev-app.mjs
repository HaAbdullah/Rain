// The development command in the real window.   npm run dev:app
//
// The same live loop as `npm run dev` (a copy of your data, hot reload, the backend rebuilt on a
// Rust change), shown in a real Rain window instead of a browser tab. Use it for what only the
// window has: maximise, zoom, dragging, hide and summon. It is a separate app for Windows (its
// own lock and WebView profile), so the installed Rain keeps running next to it. It stays open
// when it loses focus, and Ctrl+` belongs to the installed Rain while that one runs.
//
// When `npm run dev` already runs (port 1420 answers), the window attaches to it.
import { spawn } from 'node:child_process'
import { prepareMyData } from './lib/mydata.mjs'

const DEV_URL = 'http://localhost:1420'

async function devServerRuns() {
  try {
    await fetch(DEV_URL, { signal: AbortSignal.timeout(800) })
    return true
  } catch {
    return false
  }
}

const attached = await devServerRuns()
if (attached) {
  console.log(`dev:app: ${DEV_URL} already runs; the window attaches to it`)
} else {
  const copy = prepareMyData()
  if (copy) process.env.RAIN_DATA_DIR = copy
}

// `node scripts/dev.mjs` and not `npm run dev`: the copy is made once, here, and both the
// backend and the window read its folder from RAIN_DATA_DIR.
const config = JSON.stringify({ build: { beforeDevCommand: attached ? '' : 'node scripts/dev.mjs' } })
// `shell: true` because npx is a .cmd file on Windows; the JSON is quoted for cmd.exe.
const tauri = spawn(`npx tauri dev --config "${config.replaceAll('"', '\\"')}"`, { stdio: 'inherit', shell: true })
tauri.on('exit', (code) => process.exit(code ?? 0))
