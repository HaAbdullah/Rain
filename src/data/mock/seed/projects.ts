// Sample projects under C:\Dev. Typical: 11 (3 clearly most active, 2 open,
// "old-blog" has a missing folder). Busy adds generated ones up to 40.
// sessionIds start empty: the store links them to the seeded sessions.

import type { EnvStep, Project } from '../../types'
import { DAY, HOUR, MINUTE, ago, brand, createRng, letter, lucide } from '../engine'
import type { Rng } from '../engine'
import type { Scenario } from '../options'

type Activity = 'high' | 'mid' | 'low' | 'none'
type EnvPreset = 'web' | 'rust' | 'plain'

interface ProjectRow {
  name: string
  branch: string
  dirty: number
  activity: Activity
  env: EnvPreset
  /** How long ago it was last opened, in ms. */
  opened: number
  open?: boolean
  missing?: boolean
}

const PROJECTS: ProjectRow[] = [
  { name: 'weather-station', branch: 'feat/rain-gauge', dirty: 3, activity: 'high', env: 'web', opened: 25 * MINUTE, open: true },
  { name: 'recipe-box', branch: 'fix/login-redirect', dirty: 5, activity: 'high', env: 'web', opened: 50 * MINUTE, open: true },
  { name: 'sensor-hub', branch: 'main', dirty: 1, activity: 'high', env: 'rust', opened: 3 * HOUR },
  { name: 'budget-tracker', branch: 'feat/csv-import', dirty: 2, activity: 'mid', env: 'web', opened: 1 * DAY },
  { name: 'pixel-garden', branch: 'main', dirty: 0, activity: 'mid', env: 'web', opened: 2 * DAY },
  { name: 'trail-notes', branch: 'draft/maps', dirty: 7, activity: 'mid', env: 'web', opened: 4 * DAY },
  { name: 'home-dashboard', branch: 'main', dirty: 0, activity: 'low', env: 'web', opened: 9 * DAY },
  { name: 'dotfiles', branch: 'main', dirty: 1, activity: 'low', env: 'plain', opened: 13 * DAY },
  { name: 'chess-clock', branch: 'main', dirty: 0, activity: 'low', env: 'rust', opened: 21 * DAY },
  { name: 'markdown-press', branch: 'release/2.0', dirty: 0, activity: 'low', env: 'web', opened: 34 * DAY },
  { name: 'old-blog', branch: 'main', dirty: 0, activity: 'none', env: 'plain', opened: 140 * DAY, missing: true },
]

// Busy scenario: invented names made from two word lists.
const NAME_FIRST = ['paper', 'tide', 'lantern', 'copper', 'quiet', 'north', 'amber', 'signal', 'moss', 'cinder']
const NAME_SECOND = ['kite', 'table', 'press', 'relay', 'atlas', 'ledger', 'works', 'garden', 'bridge', 'index']
const BUSY_BRANCHES = ['main', 'main', 'main', 'dev', 'feat/search', 'fix/build', 'chore/deps']
const BUSY_PROJECT_COUNT = 40

const EDITOR: EnvStep = { kind: 'editor', label: 'VS Code', icon: letter('VS Code'), appId: 'app-vs-code' }
const CLAUDE: EnvStep = { kind: 'claude', label: 'Claude session', icon: brand('claude'), command: 'claude' }

const ENVIRONMENTS: Record<EnvPreset, EnvStep[]> = {
  web: [
    EDITOR,
    { kind: 'terminal', label: 'Dev server', icon: lucide('square-terminal'), appId: 'app-terminal', command: 'npm run dev' },
    { kind: 'browser', label: 'Browser preview', icon: brand('googlechrome'), appId: 'app-chrome', url: 'http://localhost:5173' },
    CLAUDE,
  ],
  rust: [
    EDITOR,
    { kind: 'terminal', label: 'Watch build', icon: lucide('square-terminal'), appId: 'app-terminal', command: 'cargo watch -x run' },
    CLAUDE,
  ],
  plain: [
    EDITOR,
    { kind: 'terminal', label: 'Terminal', icon: lucide('square-terminal'), appId: 'app-terminal' },
  ],
}

export function seed(scenario: Scenario): Project[] {
  if (scenario === 'empty') return []
  const rng = createRng('projects')
  const rows = scenario === 'busy' ? [...PROJECTS, ...generatedRows(rng, BUSY_PROJECT_COUNT - PROJECTS.length)] : PROJECTS
  return rows.map((row) => toProject(row, rng))
}

function toProject(row: ProjectRow, rng: Rng): Project {
  const commits14d = commits(row.activity, rng)
  return {
    id: `proj-${row.name}`,
    name: row.name,
    path: `C:\\Dev\\${row.name}`,
    icon: letter(row.name),
    branch: row.branch,
    dirtyFiles: row.dirty,
    commits14d,
    lastOpenedAt: ago(row.opened),
    activityScore: activityScore(commits14d),
    // Copied so no two projects share one step object.
    environment: ENVIRONMENTS[row.env].map((step) => ({ ...step })),
    sessionIds: [],
    open: row.open ?? false,
    pinned: false,
    missing: row.missing ?? false,
  }
}

/** Commits per day for the last 14 days, oldest first. */
function commits(activity: Activity, rng: Rng): number[] {
  return Array.from({ length: 14 }, () => {
    switch (activity) {
      case 'high': return rng.int(2, 9)
      case 'mid': return rng.chance(0.5) ? rng.int(1, 3) : 0
      case 'low': return rng.chance(0.15) ? 1 : 0
      case 'none': return 0
    }
  })
}

/** Recent commits count for more than old ones. */
function activityScore(commits14d: number[]): number {
  const weighted = commits14d.reduce((sum, count, day) => sum + (count * (day + 1)) / 14, 0)
  return Math.round(weighted)
}

function generatedRows(rng: Rng, count: number): ProjectRow[] {
  const names = NAME_FIRST.flatMap((first) => NAME_SECOND.map((second) => `${first}-${second}`))
  return rng.shuffle(names).slice(0, count).map((name, i) => ({
    name,
    branch: rng.pick(BUSY_BRANCHES),
    dirty: rng.chance(0.3) ? rng.int(1, 6) : 0,
    activity: rng.chance(0.3) ? 'mid' : 'low',
    env: rng.pick(['web', 'rust', 'plain'] as const),
    // Spread over the past year, after the hand-written ones.
    opened: (5 + i * 9 + rng.int(0, 6)) * DAY,
  }))
}
