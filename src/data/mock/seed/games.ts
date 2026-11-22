// Sample game library. Typical: 28 invented titles (Steam 16, Epic 8, custom 4;
// 20 installed; 2 without a cover; 6 played in the last week). Busy adds
// generated titles up to 180. Covers are generated pictures, never real artwork.

import type { Game, GameSource } from '../../types'
import { DAY, HOUR, ago, createRng, slug } from '../engine'
import type { Rng } from '../engine'
import { coverFor } from '../covers'
import type { Scenario } from '../options'

const GB = 1024 ** 3

interface GameRow {
  title: string
  source: GameSource
  /** Size on disk in GB when installed; leave out for a game that is not installed. */
  sizeGb?: number
  minutes: number
  /** How long ago it was last played, in ms; leave out for "never". */
  played?: number
  noCover?: boolean
}

const GAMES: GameRow[] = [
  { title: 'Ashen Harbor', source: 'steam', sizeGb: 38, minutes: 42 * 60, played: 5 * HOUR },
  { title: 'Lantern Drift', source: 'steam', sizeGb: 12, minutes: 118 * 60, played: 2 * DAY },
  { title: 'Quiet Orbit', source: 'steam', sizeGb: 54, minutes: 9 * 60, played: 4 * DAY },
  { title: 'Hollow Pines', source: 'steam', sizeGb: 71, minutes: 63 * 60, played: 12 * DAY },
  { title: 'Signal Nine', source: 'steam', sizeGb: 22, minutes: 27 * 60, played: 20 * DAY },
  { title: 'Tidewalker', source: 'steam', sizeGb: 8, minutes: 4 * 60, played: 33 * DAY },
  { title: 'Glass Meridian', source: 'steam', sizeGb: 96, minutes: 211 * 60, played: 45 * DAY },
  { title: 'Ember Circuit', source: 'steam', sizeGb: 17, minutes: 15 * 60, played: 60 * DAY },
  { title: 'Northwind Saga', source: 'steam', sizeGb: 64, minutes: 88 * 60, played: 75 * DAY },
  { title: 'Paper Kingdoms', source: 'steam', sizeGb: 3, minutes: 36 * 60, played: 110 * DAY },
  { title: 'Deep Current', source: 'steam', sizeGb: 29, minutes: 35, played: 150 * DAY },
  { title: 'Ironroot', source: 'steam', minutes: 19 * 60, played: 200 * DAY },
  { title: 'Starling Run', source: 'steam', minutes: 6 * 60, played: 260 * DAY },
  { title: 'Vault of Echoes', source: 'steam', minutes: 0 },
  { title: 'Copper Skies', source: 'steam', minutes: 51 * 60, played: 320 * DAY },
  { title: 'Midnight Cartographer', source: 'steam', minutes: 0 },

  { title: 'Frostline', source: 'epic', sizeGb: 47, minutes: 33 * 60, played: 3 * DAY },
  { title: 'Salt and Circuit', source: 'epic', sizeGb: 14, minutes: 12 * 60, played: 6 * DAY },
  { title: "Rook's Gambit", source: 'epic', sizeGb: 5, minutes: 74 * 60, played: 25 * DAY },
  { title: 'Dune Racers', source: 'epic', sizeGb: 41, minutes: 8 * 60, played: 90 * DAY },
  { title: 'Low Tide Club', source: 'epic', sizeGb: 9, minutes: 3 * 60, played: 130 * DAY, noCover: true },
  { title: 'Moss Garden', source: 'epic', minutes: 0 },
  { title: 'Sky Ferry', source: 'epic', minutes: 22 * 60, played: 400 * DAY },
  { title: 'Cinder Vale', source: 'epic', minutes: 0 },

  { title: 'Pocket Atlas', source: 'custom', sizeGb: 2, minutes: 140 * 60, played: 1 * DAY },
  { title: 'Thornfield', source: 'custom', sizeGb: 6, minutes: 17 * 60, played: 15 * DAY },
  { title: 'Wren Hollow', source: 'custom', sizeGb: 1, minutes: 5 * 60, played: 50 * DAY },
  { title: 'Blackbox Arcade', source: 'custom', sizeGb: 0.8, minutes: 61 * 60, played: 180 * DAY, noCover: true },
]

// Busy scenario: invented titles made from two word lists.
const TITLE_FIRST = ['Silent', 'Golden', 'Broken', 'Distant', 'Crimson', 'Hidden', 'Frozen', 'Wandering', 'Hollow', 'Electric',
  'Velvet', 'Rusted', 'Bright', 'Sunken']
const TITLE_SECOND = ['Lighthouse', 'Caravan', 'Engine', 'Harvest', 'Signal', 'Citadel', 'Orchard', 'Compass', 'Reactor', 'Tundra',
  'Parade', 'Archive', 'Canyon', 'Workshop']
const BUSY_GAME_COUNT = 180

export function seed(scenario: Scenario): Game[] {
  if (scenario === 'empty') return []
  const rows = scenario === 'busy' ? [...GAMES, ...generatedRows(BUSY_GAME_COUNT - GAMES.length)] : GAMES
  return rows.map(toGame)
}

function toGame(row: GameRow): Game {
  return {
    id: `game-${slug(row.title)}`,
    title: row.title,
    source: row.source,
    coverUrl: row.noCover ? undefined : coverFor(row.title),
    installed: row.sizeGb !== undefined,
    running: false,
    sizeBytes: row.sizeGb === undefined ? undefined : Math.round(row.sizeGb * GB),
    playtimeMinutes: row.minutes,
    lastPlayedAt: row.played === undefined ? undefined : ago(row.played),
    pinned: false,
    hidden: false,
  }
}

function generatedRows(count: number): GameRow[] {
  const rng = createRng('games')
  const titles = TITLE_FIRST.flatMap((first) => TITLE_SECOND.map((second) => `${first} ${second}`))
  return rng.shuffle(titles).slice(0, count).map((title) => generatedRow(title, rng))
}

function generatedRow(title: string, rng: Rng): GameRow {
  const source: GameSource = rng.pick(['steam', 'steam', 'steam', 'steam', 'steam', 'steam', 'epic', 'epic', 'epic', 'custom'])
  const neverPlayed = rng.chance(0.2)
  return {
    title,
    source,
    // Custom games are always on disk; store games are installed a little over half the time.
    sizeGb: source === 'custom' || rng.chance(0.55) ? rng.int(1, 110) : undefined,
    minutes: neverPlayed ? 0 : rng.int(20, 180 * 60),
    played: neverPlayed ? undefined : rng.int(8, 700) * DAY,
  }
}
