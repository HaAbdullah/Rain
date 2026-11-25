// The development command on a copy of your own data.   npm run dev:mydata
//
// `npm run dev` starts with empty sandbox folders, so the screens are empty. This command first
// copies your real data folder (pools, pins, habits, games, settings) to a throwaway folder and
// runs the same development command on the copy. Screens look like your real Rain, edits to the
// code show at once, and nothing a half-finished change does can reach your real files.
//
// The habit notes are copied too (into `vault` inside the copy), so ticking a habit in the
// development window writes to the copy, never to your Obsidian vault.
import { cpSync, existsSync, mkdirSync, readFileSync, readdirSync, rmSync, appendFileSync } from 'node:fs'
import { join } from 'node:path'
import { REPO } from './backend.mjs'

const COPY = join(REPO, '.rain-dev', 'mydata')

/** A single-quoted or double-quoted TOML text value of `key`, or undefined. Enough for Rain's own files. */
function tomlText(text, key) {
  const line = text.split(/\r?\n/).find((l) => l.trimStart().startsWith(`${key} `) || l.trimStart().startsWith(`${key}=`))
  const literal = line?.match(/=\s*'([^']*)'/)
  if (literal) return literal[1]
  const basic = line?.match(/=\s*"((?:[^"\\]|\\.)*)"/)
  return basic ? JSON.parse(`"${basic[1]}"`) : undefined
}

function realDataDir() {
  const pointer = join(process.env.APPDATA ?? '', 'Rain', 'machine.toml')
  if (!existsSync(pointer)) return undefined
  return tomlText(readFileSync(pointer, 'utf8'), 'data_dir')
}

/** The vault Rain uses: the override in settings.toml, else the only vault Obsidian knows. */
function realVault(settingsText) {
  const override = tomlText(settingsText, 'vault_path')
  if (override) return override
  const obsidian = join(process.env.APPDATA ?? '', 'obsidian', 'obsidian.json')
  if (!existsSync(obsidian)) return undefined
  const vaults = Object.values(JSON.parse(readFileSync(obsidian, 'utf8')).vaults ?? {})
  return vaults.length === 1 ? vaults[0].path : undefined
}

const source = realDataDir()
if (!source || !existsSync(source)) {
  console.error('dev:mydata: no data folder found (is Rain installed and started once?). Use `npm run dev`.')
  process.exit(1)
}

rmSync(COPY, { recursive: true, force: true })
mkdirSync(COPY, { recursive: true })
for (const name of readdirSync(source)) {
  if (name.endsWith('.toml')) cpSync(join(source, name), join(COPY, name))
}

// The habit notes: only the note folder Rain uses, copied into the data copy. A sandbox run may
// write to a vault only when it lies inside its data folder, so this is what makes ticks work.
const settingsPath = join(COPY, 'settings.toml')
const settingsText = existsSync(settingsPath) ? readFileSync(settingsPath, 'utf8') : ''
const vault = realVault(settingsText)
const noteFolder = tomlText(settingsText, 'daily_note_folder')
const vaultCopy = join(COPY, 'vault')
mkdirSync(noteFolder ? join(vaultCopy, noteFolder) : vaultCopy, { recursive: true })
if (vault && noteFolder && existsSync(join(vault, noteFolder))) {
  cpSync(join(vault, noteFolder), join(vaultCopy, noteFolder), { recursive: true })
}
if (!tomlText(settingsText, 'vault_path')) appendFileSync(settingsPath, `\nvault_path = '${vaultCopy}'\n`)

console.log(`dev:mydata: running on a copy of ${source} in ${COPY}`)
process.env.RAIN_DATA_DIR = COPY
await import('./dev.mjs')
