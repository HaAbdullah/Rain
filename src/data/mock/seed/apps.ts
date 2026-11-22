// Sample apps and pools. Typical: 24 apps (6 running, 4 pinned) and 4 pools.
// Busy adds generated tools up to 150 apps and 9 pools. Empty has neither.

import type { AppEntry, IconRef, ID, ISODateTime, Pool, PoolMember } from '../../types'
import { DAY, HOUR, MINUTE, ago, brand, createRng, letter, lucide, slug } from '../engine'
import type { Scenario } from '../options'

interface AppRow {
  name: string
  icon: IconRef
  exe: string
  keywords: string[]
  running?: boolean
  pinned?: boolean
  /** How long ago it was last launched, in ms. */
  launched?: number
}

// Microsoft apps have no brand mark in simple-icons, so they use letter tiles.
const APPS: AppRow[] = [
  { name: 'VS Code', icon: letter('VS Code'), exe: 'C:\\Program Files\\Microsoft VS Code\\Code.exe', keywords: ['editor', 'code', 'ide'], running: true, pinned: true, launched: 25 * MINUTE },
  { name: 'Terminal', icon: letter('Terminal'), exe: 'C:\\Program Files\\WindowsApps\\Terminal\\wt.exe', keywords: ['shell', 'console', 'powershell'], running: true, pinned: true, launched: 40 * MINUTE },
  { name: 'Chrome', icon: brand('googlechrome'), exe: 'C:\\Program Files\\Google\\Chrome\\Application\\chrome.exe', keywords: ['browser', 'web'], running: true, pinned: true, launched: 2 * HOUR },
  { name: 'Obsidian', icon: brand('obsidian'), exe: 'C:\\Program Files\\Obsidian\\Obsidian.exe', keywords: ['notes', 'vault', 'markdown'], running: true, pinned: true, launched: 5 * HOUR },
  { name: 'Discord', icon: brand('discord'), exe: 'C:\\Program Files\\Discord\\Discord.exe', keywords: ['chat', 'voice'], running: true, launched: 7 * HOUR },
  { name: 'Steam', icon: brand('steam'), exe: 'C:\\Program Files (x86)\\Steam\\steam.exe', keywords: ['games', 'store'], running: true, launched: 1 * DAY },
  { name: 'Epic Games Launcher', icon: brand('epicgames'), exe: 'C:\\Program Files\\Epic Games\\Launcher\\EpicGamesLauncher.exe', keywords: ['games', 'store'], launched: 9 * DAY },
  { name: 'Claude', icon: brand('claude'), exe: 'C:\\Program Files\\Claude\\Claude.exe', keywords: ['ai', 'assistant'], launched: 3 * DAY },
  { name: 'GitHub Desktop', icon: brand('github'), exe: 'C:\\Program Files\\GitHub Desktop\\GitHubDesktop.exe', keywords: ['git', 'repository'], launched: 6 * DAY },
  { name: 'Google Calendar', icon: brand('googlecalendar'), exe: 'C:\\Program Files\\Google\\Chrome\\Application\\chrome_proxy.exe', keywords: ['agenda', 'events'] },
  { name: 'File Explorer', icon: letter('File Explorer'), exe: 'C:\\Windows\\explorer.exe', keywords: ['files', 'folders'], launched: 2 * DAY },
  { name: 'Notepad', icon: letter('Notepad'), exe: 'C:\\Windows\\System32\\notepad.exe', keywords: ['text'] },
  { name: 'Calculator', icon: letter('Calculator'), exe: 'C:\\Windows\\System32\\calc.exe', keywords: ['math'] },
  { name: 'Paint', icon: letter('Paint'), exe: 'C:\\Windows\\System32\\mspaint.exe', keywords: ['draw'] },
  { name: 'Task Manager', icon: letter('Task Manager'), exe: 'C:\\Windows\\System32\\Taskmgr.exe', keywords: ['processes', 'performance'] },
  { name: 'Word', icon: letter('Word'), exe: 'C:\\Program Files\\Microsoft Office\\root\\Office16\\WINWORD.EXE', keywords: ['documents', 'office'], launched: 12 * DAY },
  { name: 'Excel', icon: letter('Excel'), exe: 'C:\\Program Files\\Microsoft Office\\root\\Office16\\EXCEL.EXE', keywords: ['spreadsheet', 'office'], launched: 20 * DAY },
  { name: 'Edge', icon: letter('Edge'), exe: 'C:\\Program Files (x86)\\Microsoft\\Edge\\Application\\msedge.exe', keywords: ['browser', 'web'] },
  { name: 'Capture tool', icon: lucide('video'), exe: 'C:\\Tools\\Capture\\capture.exe', keywords: ['record', 'stream', 'screen'] },
  { name: 'Music player', icon: lucide('music'), exe: 'C:\\Tools\\Music\\player.exe', keywords: ['audio', 'songs'], launched: 1 * DAY },
  { name: 'Image editor', icon: lucide('image'), exe: 'C:\\Tools\\ImageEditor\\editor.exe', keywords: ['photo', 'graphics'], launched: 12 * DAY },
  { name: 'Video player', icon: lucide('film'), exe: 'C:\\Tools\\VideoPlayer\\player.exe', keywords: ['media', 'movie'] },
  { name: 'Database browser', icon: lucide('database'), exe: 'C:\\Tools\\DbBrowser\\dbbrowser.exe', keywords: ['sql', 'tables'], launched: 4 * DAY },
  { name: 'Password manager', icon: lucide('key-round'), exe: 'C:\\Tools\\Passwords\\passwords.exe', keywords: ['secrets', 'login'] },
]

/** Its executable is "missing": every launch of this app fails. */
const BROKEN_APP = 'Capture tool'

interface PoolRow {
  name: string
  /** App names; `args` and `delayMs` are optional per member. */
  members: { app: string; args?: string; delayMs?: number }[]
  pinned?: boolean
  launched?: number
}

const POOLS: PoolRow[] = [
  { name: 'Web dev', pinned: true, launched: 3 * HOUR, members: [
    { app: 'VS Code' }, { app: 'Terminal' }, { app: 'Chrome', args: '--new-window http://localhost:5173', delayMs: 2000 },
  ] },
  { name: 'Writing', pinned: true, launched: 1 * DAY, members: [{ app: 'Obsidian' }, { app: 'Music player' }] },
  { name: 'Streaming', pinned: true, launched: 6 * DAY, members: [{ app: 'Discord' }, { app: BROKEN_APP }, { app: 'Music player' }] },
  { name: 'Design review', launched: 12 * DAY, members: [{ app: 'Image editor' }, { app: 'Chrome' }, { app: 'Notepad' }, { app: 'File Explorer' }] },
]

const BUSY_POOLS: PoolRow[] = [
  { name: 'Morning', launched: 2 * DAY, members: [{ app: 'Chrome' }, { app: 'Google Calendar' }, { app: 'Obsidian' }] },
  { name: 'Gaming', launched: 4 * DAY, members: [{ app: 'Steam' }, { app: 'Discord' }] },
  { name: 'Admin', launched: 15 * DAY, members: [{ app: 'Task Manager' }, { app: 'File Explorer' }, { app: 'Terminal' }] },
  { name: 'Research', launched: 8 * DAY, members: [{ app: 'Chrome' }, { app: 'Obsidian' }, { app: 'Claude' }] },
  { name: 'Office', launched: 30 * DAY, members: [{ app: 'Word' }, { app: 'Excel' }] },
]

// Busy scenario: generic tool names made from two word lists.
const TOOL_SUBJECTS = ['Audio', 'Video', 'Photo', 'Font', 'PDF', 'Disk', 'Network', 'Backup', 'Colour', 'Markdown', 'Archive',
  'Screen', 'Clipboard', 'Batch', 'Log', 'Hex', 'Diff', 'Port', 'Subtitle', 'Icon', 'JSON']
const TOOL_KINDS = ['viewer', 'editor', 'converter', 'manager', 'monitor', 'recorder', 'cleaner']
const BUSY_APP_COUNT = 150

export interface AppSeed { apps: AppEntry[]; pools: Pool[]; brokenAppIds: ID[] }

export function seed(scenario: Scenario): AppSeed {
  if (scenario === 'empty') return { apps: [], pools: [], brokenAppIds: [] }

  const rows = scenario === 'busy' ? [...APPS, ...generatedApps(BUSY_APP_COUNT - APPS.length)] : APPS
  const poolRows = scenario === 'busy' ? [...POOLS, ...BUSY_POOLS] : POOLS
  return {
    apps: rows.map(toApp),
    pools: poolRows.map(toPool),
    brokenAppIds: [appId(BROKEN_APP)],
  }
}

function appId(name: string): ID {
  return `app-${slug(name)}`
}

function toApp(row: AppRow): AppEntry {
  return {
    id: appId(row.name),
    name: row.name,
    icon: row.icon,
    exePath: row.exe,
    keywords: row.keywords,
    pinned: row.pinned ?? false,
    running: row.running ?? false,
    lastLaunchedAt: launchedAt(row.launched),
  }
}

function toPool(row: PoolRow): Pool {
  return {
    id: `pool-${slug(row.name)}`,
    name: row.name,
    icon: letter(row.name),
    members: row.members.map(toMember),
    pinned: row.pinned ?? false,
    lastLaunchedAt: launchedAt(row.launched),
  }
}

function toMember(member: PoolRow['members'][number]): PoolMember {
  return { appId: appId(member.app), args: member.args, delayMs: member.delayMs }
}

function launchedAt(msAgo: number | undefined): ISODateTime | undefined {
  return msAgo === undefined ? undefined : ago(msAgo)
}

function generatedApps(count: number): AppRow[] {
  const rng = createRng('apps')
  const names = TOOL_SUBJECTS.flatMap((subject) => TOOL_KINDS.map((kind) => `${subject} ${kind}`))
  return rng.shuffle(names).slice(0, count).map((name) => {
    const folder = slug(name)
    return {
      name,
      icon: letter(name),
      exe: `C:\\Tools\\${folder}\\${folder}.exe`,
      keywords: name.toLowerCase().split(' '),
      // A few were used recently so "Recent" is not only the hand-written apps.
      launched: rng.chance(0.15) ? rng.int(1, 60) * DAY : undefined,
    }
  })
}
