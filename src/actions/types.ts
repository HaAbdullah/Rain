// The action model (spec 6.4). A screen says what is selected (an ActionTarget);
// the registry answers with what can be done to it (RainActions). Enter, Ctrl+Enter,
// the Ctrl+K panel and the shortcut sheet all read that one answer.
import type { useConfirm } from '@/composables/useConfirm'
import type { useLaunch } from '@/composables/useLaunch'
import type { useLogDialog } from '@/composables/useLogDialog'
import type { useNotice } from '@/composables/useNotice'
import type { RainApi } from '@/data/api'
import type {
  AppEntry,
  ClaudeSession,
  DiskNode,
  Drive,
  Game,
  HabitDay,
  ID,
  IdleJob,
  Pool,
  Project,
  ScheduledJob,
  Settings,
} from '@/data/types'
import type { useNav } from '@/nav/useNav'
import type { Shell } from '@/shell'
import type { CommandDef } from './commands'

export type ItemType =
  | 'app'
  | 'pool'
  | 'project'
  | 'game'
  | 'session'
  | 'job'
  | 'idleJob'
  | 'command'
  | 'day'
  | 'drive'
  | 'diskNode'
  | 'setting'

/** One row of the Settings screen (spec 7.9). `key` names the Settings field it edits, if any. */
export interface SettingDef {
  id: string
  group: string
  label: string
  hint?: string
  kind: 'switch' | 'select' | 'path' | 'text' | 'button' | 'info'
  key?: keyof Settings
}

export type ActionTarget =
  | { type: 'app'; id: ID; item: AppEntry }
  | { type: 'pool'; id: ID; item: Pool }
  | { type: 'project'; id: ID; item: Project }
  | { type: 'game'; id: ID; item: Game }
  | { type: 'session'; id: ID; item: ClaudeSession }
  | { type: 'job'; id: ID; item: ScheduledJob }
  | { type: 'idleJob'; id: ID; item: IdleJob }
  | { type: 'command'; id: ID; item: CommandDef }
  | { type: 'day'; id: ID; item: HabitDay }
  | { type: 'drive'; id: ID; item: Drive }
  | { type: 'diskNode'; id: ID; item: DiskNode }
  | { type: 'setting'; id: ID; item: SettingDef }

/** The target variant for one item type, for example TargetOf<'app'>. */
export type TargetOf<K extends ItemType> = Extract<ActionTarget, { type: K }>

/** Everything an action may touch. Built in one place: actions/context.ts. */
export interface ActionContext {
  /** 'home' while the Home list or the global results are showing. */
  where: 'home' | 'section'
  api: RainApi
  nav: ReturnType<typeof useNav>
  shell: Shell
  notice: ReturnType<typeof useNotice>
  confirm: ReturnType<typeof useConfirm>
  launch: ReturnType<typeof useLaunch>['launch']
  openLog: ReturnType<typeof useLogDialog>['open']
}

export interface RainAction {
  id: string
  label: string
  /** Lucide icon name (kebab-case). */
  icon: string
  /** primary = Enter, secondary = Ctrl+Enter, more = only in the Ctrl+K panel. */
  slot: 'primary' | 'secondary' | 'more'
  /** An extra chord that is live while the target is selected, for example 'Ctrl+KeyY'. */
  keys?: string
  /** Drawn last and in the destructive colour. Must ask through `ctx.confirm` before it acts. */
  danger?: boolean
  run(ctx: ActionContext): void | Promise<void>
}

export type ActionProvider<T extends ActionTarget = ActionTarget> = (
  target: T,
  ctx: ActionContext,
) => RainAction[]
