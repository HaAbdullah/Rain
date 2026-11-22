// The route table (spec 3.4). There is no router library: a route is a name,
// an optional sub-view and optional params, and `useNav` holds the current one.
// The rail, the keymap, the "Go to" commands and the browser hash all read this table.
import type { Component } from 'vue'

export type RouteName =
  | 'home'
  | 'apps'
  | 'projects'
  | 'games'
  | 'workflows'
  | 'life'
  | 'disk'
  | 'gameMode'
  | 'settings'

export interface RouteDef {
  name: RouteName
  path: string
  title: string
  /** Lucide icon name (kebab-case). */
  icon: string
  /** Key combo in keymap notation, for example 'Ctrl+Digit1'. */
  combo: string
  rail: 'top' | 'bottom'
  subs?: readonly string[]
  defaultSub?: string
  /** Whether going here adds the scope chip. False for Home and Game mode. */
  filterable: boolean
  /** Lazy import of the screen, so each screen is its own chunk. */
  component: () => Promise<{ default: Component }>
}

export interface Route {
  name: RouteName
  sub?: string
  params?: Record<string, string>
}

/** In rail order: the seven sections, then the two bottom entries. */
export const routes: readonly RouteDef[] = [
  {
    name: 'home',
    path: '/',
    title: 'Home',
    icon: 'house',
    combo: 'Ctrl+Digit1',
    rail: 'top',
    filterable: false,
    component: () => import('@/features/home/HomeScreen.vue'),
  },
  {
    name: 'apps',
    path: '/apps',
    title: 'Apps',
    icon: 'layout-grid',
    combo: 'Ctrl+Digit2',
    rail: 'top',
    filterable: true,
    component: () => import('@/features/apps/AppsScreen.vue'),
  },
  {
    name: 'projects',
    path: '/projects',
    title: 'Projects',
    icon: 'folder-git-2',
    combo: 'Ctrl+Digit3',
    rail: 'top',
    filterable: true,
    component: () => import('@/features/projects/ProjectsScreen.vue'),
  },
  {
    name: 'games',
    path: '/games',
    title: 'Games',
    icon: 'gamepad-2',
    combo: 'Ctrl+Digit4',
    rail: 'top',
    subs: ['all', 'steam', 'epic', 'custom', 'installed'],
    defaultSub: 'all',
    filterable: true,
    component: () => import('@/features/games/GamesScreen.vue'),
  },
  {
    name: 'workflows',
    path: '/workflows/:sub',
    title: 'Workflows',
    icon: 'workflow',
    combo: 'Ctrl+Digit5',
    rail: 'top',
    subs: ['sessions', 'jobs', 'idle'],
    defaultSub: 'sessions',
    filterable: true,
    component: () => import('@/features/workflows/WorkflowsScreen.vue'),
  },
  {
    name: 'life',
    path: '/life/:sub',
    title: 'Life',
    icon: 'calendar-check',
    combo: 'Ctrl+Digit6',
    rail: 'top',
    subs: ['month', 'year'],
    defaultSub: 'month',
    filterable: true,
    component: () => import('@/features/life/LifeScreen.vue'),
  },
  {
    name: 'disk',
    path: '/disk',
    title: 'Disk',
    icon: 'hard-drive',
    combo: 'Ctrl+Digit7',
    rail: 'top',
    filterable: true,
    component: () => import('@/features/disk/DiskScreen.vue'),
  },
  {
    name: 'gameMode',
    path: '/game-mode',
    title: 'Game mode',
    icon: 'circle-power',
    combo: 'Ctrl+KeyG',
    rail: 'bottom',
    filterable: false,
    component: () => import('@/features/game-mode/GameModeScreen.vue'),
  },
  {
    name: 'settings',
    path: '/settings',
    title: 'Settings',
    icon: 'settings',
    combo: 'Ctrl+Comma',
    rail: 'bottom',
    filterable: true,
    component: () => import('@/features/settings/SettingsScreen.vue'),
  },
]

export const routeByName = Object.fromEntries(routes.map((def) => [def.name, def])) as Record<
  RouteName,
  RouteDef
>

/** '/workflows/jobs' for { name: 'workflows', sub: 'jobs' }. Params are not part of the path. */
export function pathOf(route: Route): string {
  const def = routeByName[route.name]
  return def.path.replace(':sub', route.sub ?? def.defaultSub ?? '')
}

/** The reverse of `pathOf`. An unknown path gives Home. */
export function routeOfPath(path: string): Route {
  const [first = '', second] = path.split('/').filter((part) => part !== '')
  const def = routes.find((candidate) => candidate.path.split('/')[1] === first)
  if (!def) return { name: 'home' }
  if (!def.subs) return { name: def.name }
  return { name: def.name, sub: second && def.subs.includes(second) ? second : def.defaultSub }
}
