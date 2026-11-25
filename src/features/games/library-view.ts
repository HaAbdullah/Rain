// The tab and error logic of the Games screen as plain functions, so that it can be tested
// without mounting the screen (spec 7.4).
import type { Game, GameSource, GameSourceStatus } from '@/data/types'
import { SOURCE_LABEL } from './gameSource'

export const SUBS = ['all', 'steam', 'epic', 'riot', 'custom', 'installed', 'hidden'] as const
export type Sub = (typeof SUBS)[number]

/**
 * The tabs on screen. A source that is off (not installed, switched off in Settings) has no tab;
 * Hidden exists only while at least one game is hidden.
 */
export function visibleSubs(sources: readonly GameSourceStatus[], hiddenCount: number): Sub[] {
  return SUBS.filter((id) => {
    if (id === 'hidden') return hiddenCount > 0
    return !sources.some((status) => status.source === id && status.state === 'off')
  })
}

/** The tab the route asks for, or All when it is not on screen (for example Hidden after the last game was shown). */
export function resolveSub(wanted: string | undefined, visible: readonly Sub[]): Sub {
  const known = SUBS.find((id) => id === wanted) ?? 'all'
  return visible.includes(known) ? known : 'all'
}

/** Whether a game belongs to a tab. The Hidden tab shows what the other tabs leave out. */
export function inSub(game: Game, id: Sub): boolean {
  if (id === 'all' || id === 'hidden') return true
  return id === 'installed' ? game.installed : game.source === id
}

/** The games a tab lists before the filter text is applied. */
export function gamesOfSub(games: readonly Game[], id: Sub): Game[] {
  return games.filter((game) => (id === 'hidden' ? game.hidden : !game.hidden) && inSub(game, id))
}

export function failedSources(sources: readonly GameSourceStatus[]): GameSourceStatus[] {
  return sources.filter((status) => status.state === 'failed')
}

/** One message per failed source that the current tab would have shown, or one for the whole library. */
export function errorMessages(
  sources: readonly GameSourceStatus[],
  sub: Sub,
  libraryFailed: boolean,
): string[] {
  if (libraryFailed) return ['Games unavailable']
  return failedSources(sources)
    .filter((status) => sub === 'all' || sub === 'installed' || sub === status.source)
    .map((status) => status.message ?? `${SOURCE_LABEL[status.source as GameSource]} library unavailable`)
}
