// Sample drives and the generated folder tree a scan returns: about 1,500 nodes
// in Typical and about 8,000 in Busy, at most 6 levels deep, with all 8 file
// categories. A folder's size is always the exact sum of its children.

import type { DiskNode, Drive, FileCategory } from '../../types'
import { createRng } from '../engine'
import type { Rng } from '../engine'
import type { Scenario } from '../options'

const GB = 1024 ** 3

// ---- drives ----

const DRIVES: Drive[] = [
  { id: 'drive-c', letter: 'C', label: 'System', totalBytes: 931 * GB, usedBytes: 412 * GB },
  { id: 'drive-d', letter: 'D', label: 'Data', totalBytes: 1863 * GB, usedBytes: 1210 * GB },
]
const BUSY_DRIVES: Drive[] = [
  { id: 'drive-e', letter: 'E', label: 'Archive', totalBytes: 3726 * GB, usedBytes: 2048 * GB },
]

/** Scans of this drive always stop with "Access denied". */
export const DENIED_DRIVE_ID = 'drive-d'

/** A scan reports far more files than the sample tree holds, like a real drive would. */
const AVERAGE_FILE_BYTES = 2.4 * 1024 ** 2

// ---- file types ----

const CATEGORY_BY_EXTENSION: Record<string, FileCategory> = {
  mp4: 'video', mkv: 'video', mov: 'video', bik: 'video',
  jpg: 'image', png: 'image', raw: 'image',
  mp3: 'audio', flac: 'audio', ogg: 'audio', wav: 'audio',
  zip: 'archive', '7z': 'archive', iso: 'archive', pak: 'archive',
  ts: 'code', js: 'code', vue: 'code', css: 'code', rs: 'code', json: 'code', toml: 'code',
  md: 'document', pdf: 'document', docx: 'document', xlsx: 'document', txt: 'document',
  exe: 'app', dll: 'app', msi: 'app',
}

// ---- folder profiles ----
// A profile says what a generated folder contains. In a name, a run of "#" becomes
// a number with that many digits. Weights are relative sizes inside one folder.

type Range = [min: number, max: number]
/**
 * With `ext`, a file is a random name plus a random extension. Without it the
 * names are whole file names ('package.json') and each is used at most once.
 */
interface FileRule { count: Range; names: readonly string[]; ext?: readonly string[]; weight: Range }
interface DirRule { count: Range; names: readonly string[]; profile: ProfileName; weight: Range }
interface Profile { files?: FileRule[]; dirs?: DirRule[] }

type ProfileName =
  | 'system' | 'fonts' | 'logs' | 'programs' | 'program'
  | 'documents' | 'downloads' | 'videos' | 'backups'
  | 'photos' | 'photoYear' | 'album' | 'music' | 'record'
  | 'projects' | 'webProject' | 'webSource' | 'webSourceFolder' | 'rustProject' | 'rustSource' | 'rustModule' | 'gitData'
  | 'nodeModules' | 'package' | 'packageDist' | 'rustTarget' | 'rustBuild' | 'rustDeps'
  | 'games' | 'game' | 'gameData' | 'gameMovies' | 'gameAudio'

const APP_FOLDERS = ['Code Editor', 'Web Browser', 'Notes', 'Chat', 'Media Player', 'Image Editor', 'Office Suite',
  'Game Launcher', 'Database Tools', 'Backup Agent', 'Print Tools', 'Common Files', 'Runtime', 'Screen Recorder']
const WEB_PROJECTS = ['weather-station', 'recipe-box', 'budget-tracker', 'pixel-garden', 'trail-notes', 'home-dashboard', 'markdown-press']
const RUST_PROJECTS = ['sensor-hub', 'chess-clock', 'tile-forge']
const PACKAGES = ['vite', 'vue', 'typescript', 'rollup', 'esbuild', 'postcss', 'tailwindcss', 'eslint', 'prettier', 'vitest',
  'lodash', 'date-utils', 'markdown-parser', 'yaml']
const GAME_FOLDERS = ['Ashen Harbor', 'Lantern Drift', 'Quiet Orbit', 'Hollow Pines', 'Signal Nine', 'Glass Meridian',
  'Northwind Saga', 'Frostline', 'Dune Racers', 'Pocket Atlas']
const ALBUMS = ['Morning Tapes', 'Paper Lanterns', 'Low Tide', 'Night Drive', 'Field Notes', 'Glass Houses', 'Slow Rivers',
  'Northern Lights', 'Quiet Rooms', 'Open Roads', 'Small Hours', 'Rain Songs']
const OUTINGS = ['Spring walk', 'Summer trip', 'Garden', 'Birthday', 'City weekend', 'Winter hike', 'Harvest fair']
const YEARS = ['2021', '2022', '2023', '2024', '2025', '2026']

const PROFILES: Record<ProfileName, Profile> = {
  system: { files: [
    { count: [9, 14], names: ['kernel#', 'driver#', 'shell#', 'net#', 'gfx#', 'audio#'], ext: ['dll', 'dll', 'exe', 'sys'], weight: [1, 30] },
    { count: [2, 3], names: ['catalog#', 'store#'], ext: ['dat', 'bin'], weight: [5, 60] },
  ] },
  fonts: { files: [{ count: [8, 12], names: ['Sans ##', 'Serif ##', 'Mono ##'], ext: ['ttf'], weight: [1, 3] }] },
  logs: { files: [{ count: [5, 8], names: ['setup-####', 'update-####', 'events-####'], ext: ['log'], weight: [1, 6] }] },
  programs: { dirs: [{ count: [7, 10], names: APP_FOLDERS, profile: 'program', weight: [1, 20] }] },
  program: { files: [
    { count: [3, 5], names: ['app', 'launcher', 'updater', 'core#', 'plugin#'], ext: ['exe', 'dll'], weight: [2, 40] },
    { count: [2, 4], names: ['resources#', 'locale#', 'assets#'], ext: ['pak', 'dat', 'bin'], weight: [1, 25] },
    { count: [1, 1], names: ['license', 'readme'], ext: ['txt'], weight: [0.01, 0.05] },
  ] },

  documents: { files: [
    { count: [9, 14], names: ['Budget ##', 'Notes ##', 'Report ##', 'Invoice ##', 'Manual ##', 'Plan ##'], ext: ['pdf', 'docx', 'xlsx', 'md', 'txt'], weight: [1, 12] },
  ] },
  downloads: { files: [
    { count: [4, 7], names: ['installer-##', 'setup-##', 'driver-pack-##'], ext: ['exe', 'msi'], weight: [5, 60] },
    { count: [4, 7], names: ['archive-##', 'backup-##', 'bundle-##'], ext: ['zip', '7z', 'iso'], weight: [5, 200] },
    { count: [3, 5], names: ['manual-##', 'receipt-##'], ext: ['pdf'], weight: [0.5, 4] },
  ] },
  videos: { files: [
    { count: [9, 14], names: ['Recording ##', 'Clip ##', 'Holiday ##', 'Screen capture ##'], ext: ['mp4', 'mkv', 'mov'], weight: [5, 120] },
  ] },
  backups: { files: [
    { count: [8, 12], names: ['system-image-##', 'documents-####', 'photos-####'], ext: ['7z', 'zip', 'iso'], weight: [20, 300] },
  ] },

  photos: { dirs: [{ count: [3, 4], names: YEARS, profile: 'photoYear', weight: [5, 20] }] },
  photoYear: { dirs: [{ count: [2, 4], names: OUTINGS, profile: 'album', weight: [2, 12] }] },
  album: { files: [{ count: [6, 11], names: ['IMG_####'], ext: ['jpg', 'jpg', 'jpg', 'png', 'raw'], weight: [1, 8] }] },
  music: { dirs: [{ count: [6, 9], names: ALBUMS, profile: 'record', weight: [2, 8] }] },
  record: { files: [{ count: [7, 11], names: ['Track ##'], ext: ['mp3', 'flac'], weight: [1, 6] }] },

  projects: { dirs: [
    { count: [5, 6], names: WEB_PROJECTS, profile: 'webProject', weight: [2, 12] },
    { count: [2, 2], names: RUST_PROJECTS, profile: 'rustProject', weight: [6, 20] },
  ] },
  webProject: {
    files: [{ count: [3, 4], names: ['README.md', 'package.json', 'package-lock.json', 'tsconfig.json', 'vite.config.ts'], weight: [0.01, 0.05] }],
    dirs: [
      { count: [1, 1], names: ['src'], profile: 'webSource', weight: [0.5, 2] },
      { count: [1, 1], names: ['node_modules'], profile: 'nodeModules', weight: [10, 40] },
      { count: [1, 1], names: ['.git'], profile: 'gitData', weight: [1, 8] },
    ],
  },
  webSource: {
    files: [{ count: [4, 7], names: ['main', 'app', 'config', 'routes', 'store', 'utils', 'types'], ext: ['ts', 'ts', 'vue', 'css'], weight: [1, 6] }],
    dirs: [{ count: [1, 2], names: ['components', 'lib', 'views'], profile: 'webSourceFolder', weight: [4, 14] }],
  },
  webSourceFolder: { files: [{ count: [5, 8], names: ['panel#', 'list#', 'form#', 'chart#', 'helper#'], ext: ['ts', 'vue'], weight: [1, 6] }] },
  rustProject: {
    files: [{ count: [3, 4], names: ['README.md', 'Cargo.toml', 'Cargo.lock', '.gitignore'], weight: [0.01, 0.05] }],
    dirs: [
      { count: [1, 1], names: ['src'], profile: 'rustSource', weight: [0.5, 2] },
      { count: [1, 1], names: ['target'], profile: 'rustTarget', weight: [30, 80] },
      { count: [1, 1], names: ['.git'], profile: 'gitData', weight: [1, 8] },
    ],
  },
  rustSource: {
    files: [{ count: [4, 6], names: ['main', 'lib', 'config', 'error', 'cli', 'state'], ext: ['rs'], weight: [1, 6] }],
    dirs: [{ count: [1, 2], names: ['net', 'storage', 'util'], profile: 'rustModule', weight: [4, 14] }],
  },
  rustModule: { files: [{ count: [4, 7], names: ['mod', 'reader#', 'writer#', 'parser#', 'codec#'], ext: ['rs'], weight: [1, 6] }] },
  gitData: { files: [{ count: [3, 5], names: ['pack-####', 'index', 'refs#'], ext: ['pack', 'idx'], weight: [1, 40] }] },
  nodeModules: { dirs: [{ count: [6, 9], names: PACKAGES, profile: 'package', weight: [1, 30] }] },
  package: {
    files: [{ count: [2, 4], names: ['package.json', 'index.js', 'README.md', 'LICENSE'], weight: [0.05, 1] }],
    dirs: [{ count: [1, 1], names: ['dist', 'lib'], profile: 'packageDist', weight: [5, 20] }],
  },
  packageDist: { files: [{ count: [3, 5], names: ['index', 'chunk-####', 'runtime', 'cli'], ext: ['js', 'js', 'ts', 'map'], weight: [1, 12] }] },
  rustTarget: { dirs: [{ count: [2, 2], names: ['debug', 'release'], profile: 'rustBuild', weight: [10, 40] }] },
  rustBuild: {
    files: [{ count: [2, 3], names: ['app', 'app-test#'], ext: ['exe', 'pdb'], weight: [2, 10] }],
    dirs: [{ count: [1, 1], names: ['deps'], profile: 'rustDeps', weight: [20, 60] }],
  },
  rustDeps: { files: [{ count: [6, 9], names: ['libserde-####', 'libtokio-####', 'libregex-####', 'libcore-####'], ext: ['rlib', 'rmeta'], weight: [1, 20] }] },

  games: { dirs: [{ count: [6, 8], names: GAME_FOLDERS, profile: 'game', weight: [3, 40] }] },
  game: {
    files: [
      { count: [1, 1], names: ['Game', 'Launcher'], ext: ['exe'], weight: [0.2, 1] },
      { count: [1, 2], names: ['settings', 'save#'], ext: ['ini', 'dat'], weight: [0.001, 0.01] },
    ],
    dirs: [
      { count: [1, 1], names: ['Data'], profile: 'gameData', weight: [30, 60] },
      { count: [1, 1], names: ['Movies'], profile: 'gameMovies', weight: [5, 15] },
      { count: [1, 1], names: ['Audio'], profile: 'gameAudio', weight: [3, 10] },
    ],
  },
  gameData: { files: [{ count: [5, 8], names: ['world##', 'textures##', 'models##', 'levels##'], ext: ['pak'], weight: [5, 40] }] },
  gameMovies: { files: [{ count: [2, 4], names: ['intro', 'cutscene-##', 'credits'], ext: ['bik', 'mp4'], weight: [2, 10] }] },
  gameAudio: { files: [{ count: [4, 6], names: ['music-##', 'voices-##', 'ambience-##'], ext: ['ogg', 'wav'], weight: [1, 8] }] },
}

// ---- drive layouts ----
// The hand-written top of each tree. An entry with children is a fixed folder,
// one with a profile is a generated folder, and one with neither is a file.

interface Layout { name: string; share: number; profile?: ProfileName; children?: Layout[] }

const MEDIA: Layout = { name: 'Media', share: 20, children: [
  { name: 'Video', share: 55, profile: 'videos' },
  { name: 'Photos', share: 25, profile: 'photos' },
  { name: 'Music', share: 20, profile: 'music' },
] }

const SYSTEM_DRIVE: Layout[] = [
  { name: 'Windows', share: 7, children: [
    { name: 'System32', share: 45, profile: 'system' },
    { name: 'WinSxS', share: 35, profile: 'system' },
    { name: 'Fonts', share: 3, profile: 'fonts' },
    { name: 'Logs', share: 2, profile: 'logs' },
  ] },
  { name: 'Program Files', share: 10, profile: 'programs' },
  { name: 'Program Files (x86)', share: 4, profile: 'programs' },
  { name: 'Users', share: 12, children: [
    { name: 'Public', share: 1, children: [
      { name: 'Documents', share: 4, profile: 'documents' },
      { name: 'Downloads', share: 30, profile: 'downloads' },
      { name: 'Pictures', share: 20, profile: 'photos' },
      { name: 'Music', share: 16, profile: 'music' },
      { name: 'Videos', share: 30, profile: 'videos' },
    ] },
  ] },
  { name: 'Dev', share: 9, profile: 'projects' },
  { name: 'Games', share: 34, profile: 'games' },
  MEDIA,
  { name: 'pagefile.sys', share: 3 },
  { name: 'hiberfil.sys', share: 1 },
]

const DATA_DRIVE: Layout[] = [
  { ...MEDIA, share: 45 },
  { name: 'Backups', share: 30, profile: 'backups' },
  { name: 'Games', share: 20, profile: 'games' },
  { name: 'Documents', share: 5, profile: 'documents' },
]

const MAX_DEPTH = 6
/** Busy multiplies the number of files per folder to reach about 8,000 nodes. */
const FILE_DENSITY: Record<Scenario, number> = { typical: 1, empty: 1, failing: 1, busy: 7.9 }

// ---- seed ----

export function seed(scenario: Scenario): Drive[] {
  return scenario === 'busy' ? [...DRIVES, ...BUSY_DRIVES] : DRIVES
}

/** How many files a finished scan of this drive reports. */
export function fileCount(drive: Drive): number {
  return Math.round(drive.usedBytes / AVERAGE_FILE_BYTES)
}

/** The folder tree of a drive. The same drive and scenario always give the same tree. */
export function buildTree(drive: Drive, scenario: Scenario): DiskNode {
  const builder: Builder = { rng: createRng(`disk:${drive.id}`), density: FILE_DENSITY[scenario] }
  const layout = drive.letter === 'C' ? SYSTEM_DRIVE : DATA_DRIVE
  const root: Draft = { name: `${drive.letter}:`, weight: 1, children: layout.map((entry) => draftLayout(entry, 1, builder)) }
  return toNode(root, '', drive.usedBytes)
}

// Step 1 builds the tree with relative weights only. Step 2 (toNode) hands out the
// bytes from the top down, which is what keeps every folder equal to its children.

interface Builder { rng: Rng; density: number }
/** A node before it has a size. No children means it is a file. */
interface Draft { name: string; weight: number; children?: Draft[] }

function draftLayout(entry: Layout, depth: number, builder: Builder): Draft {
  if (entry.children) {
    return { name: entry.name, weight: entry.share, children: entry.children.map((child) => draftLayout(child, depth + 1, builder)) }
  }
  if (entry.profile) return draftFolder(entry.name, entry.share, entry.profile, depth, builder)
  return { name: entry.name, weight: entry.share }
}

function draftFolder(name: string, weight: number, profileName: ProfileName, depth: number, builder: Builder): Draft {
  const { rng } = builder
  const profile = PROFILES[profileName]
  const taken = new Set<string>()
  const children: Draft[] = []

  // A folder at the deepest folder level holds files only.
  const dirRules = depth < MAX_DEPTH - 1 ? profile.dirs ?? [] : []
  for (const rule of dirRules) {
    const names = rng.shuffle(rule.names).slice(0, rng.int(rule.count[0], rule.count[1]))
    for (const dirName of names) {
      taken.add(dirName)
      children.push(draftFolder(dirName, weightIn(rule.weight, rng), rule.profile, depth + 1, builder))
    }
  }

  for (const rule of profile.files ?? []) {
    for (const fileName of fileNames(rule, taken, builder)) {
      children.push({ name: fileName, weight: weightIn(rule.weight, rng) })
    }
  }

  return { name, weight, children }
}

function fileNames(rule: FileRule, taken: Set<string>, { rng, density }: Builder): string[] {
  const count = rng.int(rule.count[0], rule.count[1])
  const { ext } = rule
  // Whole file names exist once per folder, so a denser tree cannot have more of them.
  if (!ext) return rng.shuffle(rule.names).slice(0, count)

  return Array.from({ length: Math.max(1, Math.round(count * density)) }, () =>
    uniqueName(() => `${fillNumbers(rng.pick(rule.names), rng)}.${rng.pick(ext)}`, taken))
}

function weightIn([min, max]: Range, rng: Rng): number {
  return min + rng.next() * (max - min)
}

/** 'IMG_####' -> 'IMG_0412'. */
function fillNumbers(pattern: string, rng: Rng): string {
  return pattern.replace(/#+/, (run) => String(rng.int(1, 10 ** run.length - 1)).padStart(run.length, '0'))
}

/** Tries the generator a few times, then falls back to a numbered name. */
function uniqueName(make: () => string, taken: Set<string>): string {
  let name = make()
  for (let tries = 0; taken.has(name) && tries < 5; tries += 1) name = make()
  const base = name
  for (let n = 2; taken.has(name); n += 1) name = base.replace(/(\.[^.]+)?$/, (ext) => `-${n}${ext}`)
  taken.add(name)
  return name
}

function toNode(draft: Draft, parentPath: string, bytes: number): DiskNode {
  // The drive root is 'C:\'; everything below joins with a backslash.
  const path = parentPath === '' ? `${draft.name}\\` : `${parentPath.replace(/\\$/, '')}\\${draft.name}`
  if (!draft.children) {
    return { name: draft.name, path, sizeBytes: bytes, kind: 'file', category: categoryOf(draft.name) }
  }

  const sizes = split(bytes, draft.children.map((child) => child.weight))
  const children = draft.children
    .map((child, i) => toNode(child, path, sizes[i]))
    .sort((a, b) => b.sizeBytes - a.sizeBytes)
  return { name: draft.name, path, sizeBytes: bytes, kind: 'dir', children }
}

/** Splits `total` by weight into whole numbers that add up to exactly `total`. */
function split(total: number, weights: number[]): number[] {
  const weightSum = weights.reduce((sum, weight) => sum + weight, 0)
  const parts = weights.map((weight) => Math.floor((total * weight) / weightSum))
  const handedOut = parts.reduce((sum, part) => sum + part, 0)
  // Rounding leaves a few bytes over: the largest part takes them.
  parts[parts.indexOf(Math.max(...parts))] += total - handedOut
  return parts
}

function categoryOf(fileName: string): FileCategory {
  const extension = fileName.slice(fileName.lastIndexOf('.') + 1).toLowerCase()
  return CATEGORY_BY_EXTENSION[extension] ?? 'other'
}
