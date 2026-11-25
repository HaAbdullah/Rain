// Commands: things to run that are not items (spec 6.4). The launcher scores
// them with lib/score.ts and merges them into the result list, so "sett" finds
// Settings.
import { useCheatSheet } from '@/composables/useCheatSheet'
import { followVsCodeProblem, setFollowVsCode, setTheme, themes } from '@/composables/useTheme'
import { resetZoom, zoomIn, zoomOut } from '@/composables/useZoom'
import { poolEditor, rescanApps } from '@/features/apps/actions'
import { scanDrive } from '@/features/disk/actions'
import { habitEditor, openTodaysNote } from '@/features/life/actions'
import { routes, type RouteName } from '@/nav/routes'
import { routeCombo } from '@/nav/useRailOrder'
import { toggleKeepOpen } from './common'
import type { ActionContext, ActionProvider, TargetOf } from './types'

export interface CommandDef {
  id: string
  title: string
  /** Lucide icon name (kebab-case). */
  icon: string
  keywords: string[]
  /** The command's own shortcut, shown next to it. */
  keys?: string
  run(ctx: ActionContext): void | Promise<void>
}

/** Extra words that find a screen: "habits" finds Life. */
const ROUTE_KEYWORDS: Record<RouteName, string[]> = {
  home: ['launcher', 'start', 'pinned'],
  apps: ['pools', 'programs'],
  projects: ['code', 'dev', 'repositories'],
  games: ['library', 'steam', 'epic', 'riot', 'play'],
  workflows: ['claude', 'sessions', 'jobs', 'idle', 'cron'],
  life: ['habits', 'streak', 'vault', 'daily note'],
  hardware: ['disk', 'storage', 'space', 'drives', 'keyboard', 'mouse', 'battery', 'devices'],
  claude: ['skills', 'memory', 'instructions', 'agents', 'commands', 'plugins', 'mcp', 'hooks', 'claude.md'],
  gameMode: ['wind down', 'close everything', 'shut down', 'quit'],
  settings: ['preferences', 'options', 'configuration'],
}

const goCommands: CommandDef[] = routes.map((def) => ({
  id: `go-${def.name}`,
  // Going to the Game mode screen is what starts it, so the command says so.
  title: def.name === 'gameMode' ? 'Start game mode' : `Go to ${def.title}`,
  icon: def.icon,
  keywords: ROUTE_KEYWORDS[def.name],
  // Read each time: the key of a section follows the order of the rail.
  get keys() {
    return routeCombo(def)
  },
  run: ({ nav }) => nav.go(def.name),
}))

/** One command per theme, so the look can be switched from the list. */
const themeCommands: CommandDef[] = themes.map((entry) => ({
  id: `theme-${entry.id}`,
  title: `Theme: ${entry.name}`,
  icon: 'palette',
  keywords: ['theme', 'colours', 'colors', 'appearance', 'look', 'dark', 'light', entry.name.toLowerCase()],
  run: () => setTheme(entry.id),
}))

/** The theme of the owner's VS Code. The ids do not start with "theme-" (those are the built-in themes). */
const vsCodeThemeCommands: CommandDef[] = [
  {
    id: 'vscode-theme-follow',
    title: 'Theme: follow VS Code',
    icon: 'palette',
    keywords: ['theme', 'colours', 'colors', 'vscode', 'vs code', 'bearded', 'editor', 'match'],
    async run({ notice }) {
      await setFollowVsCode(true)
      if (followVsCodeProblem.value) notice.error(followVsCodeProblem.value)
      else notice.success('Following the VS Code theme')
    },
  },
  {
    id: 'vscode-theme-unfollow',
    title: 'Theme: stop following VS Code',
    icon: 'palette',
    keywords: ['theme', 'vscode', 'vs code', 'unfollow', 'off'],
    run: () => void setFollowVsCode(false),
  },
  {
    id: 'vscode-theme-pick',
    title: 'Theme: pick one from VS Code',
    icon: 'palette',
    keywords: ['theme', 'vscode', 'vs code', 'list', 'choose', 'bearded'],
    // The list is on the Appearance row of Settings.
    run: ({ nav }) => nav.go('settings'),
  },
]

export const commands: CommandDef[] = [
  ...goCommands,
  {
    // Disk was a screen of its own before Hardware: the old way in still works.
    id: 'go-disk',
    title: 'Go to Disk',
    icon: 'hard-drive',
    keywords: ['storage', 'space', 'treemap', 'drives'],
    run: ({ nav }) => nav.go('hardware', 'disk'),
  },
  {
    id: 'today-note',
    title: "Open today's note",
    icon: 'notebook-pen',
    keywords: ['daily note', 'journal', 'obsidian'],
    keys: 'Ctrl+KeyT',
    run: openTodaysNote,
  },
  {
    id: 'scan-c',
    title: 'Scan drive C:',
    icon: 'scan',
    keywords: ['disk', 'storage', 'space'],
    async run(ctx) {
      ctx.nav.go('hardware', 'disk')
      const drives = await ctx.api.listDrives()
      const drive = drives.find((candidate) => candidate.letter.toUpperCase().startsWith('C'))
      if (drive) await scanDrive(drive.id, ctx)
      else ctx.notice.error('Drive C: not found')
    },
  },
  {
    id: 'new-pool',
    title: 'New pool',
    icon: 'layers',
    keywords: ['apps', 'group', 'create'],
    run({ nav }) {
      // The editor dialog is mounted by the Apps screen.
      nav.go('apps')
      poolEditor.create()
    },
  },
  {
    id: 'rescan-apps',
    title: 'Rescan apps',
    icon: 'refresh-cw',
    keywords: ['apps', 'installed', 'refresh', 'new app'],
    run: (ctx) => rescanApps(ctx),
  },
  {
    id: 'new-habit',
    title: 'New habit',
    icon: 'square-check',
    keywords: ['habit', 'life', 'create', 'checkbox'],
    run({ nav }) {
      // The dialog is mounted by the Life screen.
      nav.go('life')
      habitEditor.create()
    },
  },
  {
    id: 'cheat-sheet',
    title: 'Shortcut sheet',
    icon: 'keyboard',
    keywords: ['keys', 'keyboard', 'help', 'hotkeys'],
    keys: 'F1',
    run: () => useCheatSheet().open(),
  },
  {
    id: 'keep-open',
    title: 'Keep open',
    icon: 'pin',
    keywords: ['pin window', 'stay', 'hide on blur'],
    keys: 'F10',
    run: toggleKeepOpen,
  },
  {
    id: 'maximize',
    title: 'Maximise or restore',
    icon: 'maximize',
    keywords: ['maximize', 'restore', 'fullscreen', 'full screen', 'big', 'window'],
    keys: 'F11',
    run: ({ shell }) => shell.toggleMaximize(),
  },
  {
    id: 'zoom-in',
    title: 'Zoom in',
    icon: 'zoom-in',
    keywords: ['bigger', 'larger', 'magnify', 'scale', 'text size'],
    keys: 'Ctrl+Equal',
    run: () => zoomIn(),
  },
  {
    id: 'zoom-out',
    title: 'Zoom out',
    icon: 'zoom-out',
    keywords: ['smaller', 'shrink', 'scale', 'text size'],
    keys: 'Ctrl+Minus',
    run: () => zoomOut(),
  },
  {
    id: 'zoom-reset',
    title: 'Reset zoom',
    icon: 'rotate-ccw',
    keywords: ['zoom', '100', 'default size', 'scale'],
    keys: 'Ctrl+Digit0',
    run: () => resetZoom(),
  },
  ...themeCommands,
  ...vsCodeThemeCommands,
]

/** A command has one action: run it. */
export const commandActions: ActionProvider<TargetOf<'command'>> = ({ item: command }) => [
  {
    id: 'run',
    label: 'Run',
    icon: 'play',
    slot: 'primary',
    run: (ctx) => command.run(ctx),
  },
]
