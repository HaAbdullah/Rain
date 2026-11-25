// Brand marks from simple-icons. Named imports only, so the bundle holds these
// nine icons and not the whole set. Any other brand is shown as a letter tile.

import {
  siClaude,
  siDiscord,
  siEpicgames,
  siGithub,
  siGooglecalendar,
  siGooglechrome,
  siObsidian,
  siRiotgames,
  siSteam,
} from 'simple-icons'

export interface BrandIcon {
  title: string
  /** SVG path data for a 24x24 view box. */
  path: string
}

const brands: Record<string, BrandIcon> = {
  claude: siClaude,
  discord: siDiscord,
  epicgames: siEpicgames,
  github: siGithub,
  googlecalendar: siGooglecalendar,
  googlechrome: siGooglechrome,
  obsidian: siObsidian,
  riotgames: siRiotgames,
  steam: siSteam,
}

/** The mark for a simple-icons slug ('steam', 'epicgames', ...), or undefined when Rain does not ship it. */
export function brandIcon(slug: string): BrandIcon | undefined {
  return brands[slug]
}
