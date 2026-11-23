// Icon and label of each game source, shared by the strip, the tile and the detail pane.
import type { GameSource, IconRef } from '@/data/types'

export const SOURCE_ICON: Record<GameSource, IconRef> = {
  steam: { kind: 'brand', value: 'steam' },
  epic: { kind: 'brand', value: 'epicgames' },
  custom: { kind: 'lucide', value: 'box' },
}

export const SOURCE_LABEL: Record<GameSource, string> = {
  steam: 'Steam',
  epic: 'Epic Games',
  custom: 'Local',
}
