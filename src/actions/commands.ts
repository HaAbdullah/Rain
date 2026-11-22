// Commands: things to run that are not items (spec 6.4). The launcher scores
// them with lib/score.ts and merges them into the result list, so "sett" finds
// Settings and "busy" finds the mock scenario.
import { useCheatSheet } from '@/composables/useCheatSheet'
import { mockControl } from '@/data/mock/control'
import { poolEditor } from '@/features/apps/actions'
import { scanDrive } from '@/features/disk/actions'
import { openTodaysNote } from '@/features/life/actions'
import { routes, type RouteName } from '@/nav/routes'
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
  games: ['library', 'steam', 'epic', 'play'],
  workflows: ['claude', 'sessions', 'jobs', 'idle', 'cron'],
  life: ['habits', 'calendar', 'agenda', 'vault'],
  disk: ['storage', 'space', 'treemap', 'drives'],
  gameMode: ['wind down', 'close everything', 'shut down', 'quit'],
  settings: ['preferences', 'options', 'configuration'],
}

const goCommands: CommandDef[] = routes.map((def) => ({
  id: `go-${def.name}`,
  // Going to the Game mode screen is what starts it, so the command says so.
  title: def.name === 'gameMode' ? 'Start game mode' : `Go to ${def.title}`,
  icon: def.icon,
  keywords: ROUTE_KEYWORDS[def.name],
  keys: def.combo,
  run: ({ nav }) => nav.go(def.name),
}))

type Scenario = Parameters<typeof mockControl.setScenario>[0]

const SCENARIOS: { id: Scenario; title: string }[] = [
  { id: 'typical', title: 'Typical' },
  { id: 'empty', title: 'Empty' },
  { id: 'busy', title: 'Busy' },
  { id: 'failing', title: 'Failing' },
]

const scenarioCommands: CommandDef[] = SCENARIOS.map(({ id, title }) => ({
  id: `scenario-${id}`,
  title: `Mock scenario: ${title}`,
  icon: 'flask-conical',
  keywords: ['prototype', 'sample data'],
  run({ notice }) {
    mockControl.setScenario(id)
    notice.info(`Mock scenario: ${title}`)
  },
}))

export const commands: CommandDef[] = [
  ...goCommands,
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
      ctx.nav.go('disk')
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
    id: 'fullscreen',
    title: 'Full screen',
    icon: 'maximize',
    keywords: ['fullscreen', 'maximise', 'maximize', 'big', 'window'],
    keys: 'F11',
    run: ({ shell }) => shell.toggleFullscreen(),
  },
  {
    id: 'simulate-permission',
    title: 'Simulate permission request',
    icon: 'hand',
    keywords: ['prototype', 'claude', 'waiting', 'approve'],
    run({ notice }) {
      mockControl.simulatePermissionRequest()
      notice.info('A session now waits on you')
    },
  },
  ...scenarioCommands,
  {
    id: 'reset-mock',
    title: 'Reset mock data',
    icon: 'rotate-ccw',
    keywords: ['prototype', 'sample data', 'start over'],
    run({ notice }) {
      mockControl.reset()
      notice.info('Mock data reset')
    },
  },
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
