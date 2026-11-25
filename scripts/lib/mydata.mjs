// A throwaway copy of the user's own data folder for a development run.
//
// `prepareMyData()` copies the real data folder (pools, pins, habits, games, settings) and the
// habit notes of the vault to `.rain-dev/mydata` and returns that folder. A development backend
// started with `RAIN_DATA_DIR` set to it shows the same screens as the installed Rain, and
// nothing a half-finished change does can reach the real files: ticking a habit writes to the
// copy of the notes (`vault` inside the copy), never to the Obsidian vault.
import { appendFileSync, cpSync, existsSync, mkdirSync, readFileSync, readdirSync, rmSync } from 'node:fs'
import { dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'

const REPO = join(dirname(fileURLToPath(import.meta.url)), '..', '..')
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

/** Makes the copy (replacing the one of the last run). Returns its folder, or undefined when there is no real data. */
export function prepareMyData() {
  const source = realDataDir()
  if (!source || !existsSync(source)) return undefined

  rmSync(COPY, { recursive: true, force: true })
  mkdirSync(COPY, { recursive: true })
  for (const name of readdirSync(source)) {
    if (name.endsWith('.toml')) cpSync(join(source, name), join(COPY, name))
  }
  if (existsSync(join(source, 'media'))) cpSync(join(source, 'media'), join(COPY, 'media'), { recursive: true })

  // The habit notes: only the note folder Rain uses. A sandbox run may write to a vault only
  // when it lies inside its data folder, so this is what makes ticks work.
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

  console.log(`dev: running on a copy of ${source} in ${COPY}`)
  return COPY
}
