<script setup lang="ts">
// The Games screen (spec 7.4): source strip, a grid of covers (Recent, then Library A to Z),
// the detail pane of the selected game, and the dialog that adds a custom game.
import { computed, useTemplateRef, type ComputedRef } from 'vue'
import { runPrimary } from '@/actions/context'
import type { ActionTarget } from '@/actions/types'
import EmptyState from '@/components/rain/EmptyState.vue'
import ErrorRow from '@/components/rain/ErrorRow.vue'
import ItemGrid from '@/components/rain/ItemGrid.vue'
import ScreenFrame from '@/components/rain/ScreenFrame.vue'
import ViewStrip, { type ViewStripItem } from '@/components/rain/ViewStrip.vue'
import { Skeleton } from '@/components/ui/skeleton'
import { useActionPanel } from '@/composables/useActionPanel'
import { useResource } from '@/composables/useResource'
import { useScreen } from '@/composables/useScreen'
import { useSectionQuery } from '@/composables/useSearch'
import { useSelection, type SelGroup } from '@/composables/useSelection'
import { api } from '@/data'
import type { Game, GameSource } from '@/data/types'
import { matches } from '@/lib/score'
import { useNav } from '@/nav/useNav'
import CustomGameDialog from './CustomGameDialog.vue'
import GameDetail from './GameDetail.vue'
import GameTile from './GameTile.vue'
import { SOURCE_ICON, SOURCE_LABEL } from './gameSource'

const SUBS = ['all', 'steam', 'epic', 'custom', 'installed'] as const
type Sub = (typeof SUBS)[number]

/** Columns assumed until the grid has measured itself (6 at the default window size). */
const FIRST_PAINT_COLUMNS = 6
const SKELETON_TILES = 12

/** Tile geometry, shared by the grid and its skeleton. */
const MIN_COL = 132
const GAP = 12

const nav = useNav()
const panel = useActionPanel()
const query = useSectionQuery('games')
const library = useResource('games', () => api.listGames())

const sub = computed<Sub>({
  get: () => SUBS.find((id) => id === nav.route.value.sub) ?? 'all',
  set: (id) => nav.setSub(id),
})

// ---- what is shown ----

const shown = computed<Game[]>(() => (library.data.value?.games ?? []).filter((game) => !game.hidden))

function inSub(game: Game, id: Sub): boolean {
  if (id === 'all') return true
  return id === 'installed' ? game.installed : game.source === id
}

const filtered = computed(() =>
  shown.value.filter((game) => inSub(game, sub.value) && matches(query.value, game.title)),
)

const failedSources = computed<GameSource[]>(() => library.data.value?.failedSources ?? [])

const stripItems = computed<ViewStripItem[]>(() => {
  const loaded = library.data.value !== undefined
  const count = (id: Sub) => (loaded ? shown.value.filter((game) => inSub(game, id)).length : undefined)
  const source = (id: GameSource): ViewStripItem => ({
    id,
    label: SOURCE_LABEL[id].split(' ')[0],
    icon: SOURCE_ICON[id].value,
    brand: SOURCE_ICON[id].kind === 'brand',
    count: count(id),
    warn: failedSources.value.includes(id),
  })
  return [
    { id: 'all', label: 'All', icon: 'library', count: count('all') },
    source('steam'),
    source('epic'),
    source('custom'),
    { id: 'installed', label: 'Installed', icon: 'hard-drive-download', count: count('installed') },
  ]
})

/** One message per failed source that the current view would have shown, or one for the whole library. */
const errors = computed(() => {
  if (library.error.value) return ['Games unavailable']
  return failedSources.value
    .filter((id) => sub.value === 'all' || sub.value === 'installed' || sub.value === id)
    .map((id) => `${SOURCE_LABEL[id]} library unavailable`)
})

// ---- groups and selection ----

const grid = useTemplateRef<{ columns: number }>('grid')
const columns = computed(() => grid.value?.columns ?? FIRST_PAINT_COLUMNS)

const groups = computed<SelGroup<Game>[]>(() => {
  // A filter text gives one flat list of matches; Recent would only repeat them.
  const recent =
    query.value === ''
      ? filtered.value
          .filter((game) => game.lastPlayedAt)
          .sort((a, b) => (b.lastPlayedAt ?? '').localeCompare(a.lastPlayedAt ?? ''))
          .slice(0, columns.value)
      : []
  const inRecent = new Set(recent.map((game) => game.id))
  const rest = filtered.value
    .filter((game) => !inRecent.has(game.id))
    .sort((a, b) => a.title.localeCompare(b.title))
  return [
    { id: 'recent', title: 'Recent', items: recent },
    { id: 'library', title: 'Library', items: rest },
  ]
})

const { selectedId, selected, move, page, group, select } = useSelection(groups, {
  columns,
  resetOn: query,
})

const target: ComputedRef<ActionTarget | null> = computed(() =>
  selected.value ? { type: 'game', id: selected.value.id, item: selected.value } : null,
)

useScreen({ target, move, page, group, subView: { ids: SUBS, current: sub } })

/** A strip click: only ids of the strip itself arrive. */
function pickSub(id: string) {
  const known = SUBS.find((candidate) => candidate === id)
  if (known) sub.value = known
}

function openPanel(game: Game, event: MouseEvent) {
  select(game.id)
  panel.open({ type: 'game', id: game.id, item: game }, event)
}

const isEmpty = computed(() => library.data.value !== undefined && filtered.value.length === 0)
</script>

<template>
  <ScreenFrame pane="md">
    <template #strip>
      <ViewStrip :items="stripItems" :model-value="sub" @update:model-value="pickSub" />
    </template>

    <div v-if="errors.length > 0" class="flex shrink-0 flex-col px-4 pt-3">
      <ErrorRow v-for="message in errors" :key="message" :message="message" @retry="library.reload()" />
    </div>

    <div
      v-if="library.showSkeleton.value"
      class="grid shrink-0 p-4"
      :style="{ gap: `${GAP}px`, gridTemplateColumns: `repeat(auto-fill, minmax(${MIN_COL}px, 1fr))` }"
    >
      <div v-for="n in SKELETON_TILES" :key="n">
        <Skeleton class="aspect-[2/3] rounded-lg" />
        <Skeleton class="mt-1.5 h-3 w-3/4" />
      </div>
    </div>

    <EmptyState
      v-else-if="isEmpty && query !== ''"
      icon="search-x"
      title="No match"
    />
    <EmptyState
      v-else-if="isEmpty"
      icon="gamepad-2"
      title="No games found"
      :action-label="shown.length === 0 ? 'Open settings' : undefined"
      @action="nav.go('settings')"
    />
    <ItemGrid
      v-else-if="library.data.value"
      ref="grid"
      :groups="groups"
      :selected-id="selectedId"
      :min-col="MIN_COL"
      :gap="GAP"
      @update:selected-id="select"
      @activate="runPrimary({ type: 'game', id: $event.id, item: $event })"
      @context="openPanel"
    >
      <template #tile="{ item, selected: isSelected }">
        <GameTile :game="item" :selected="isSelected" />
      </template>
    </ItemGrid>

    <template #pane>
      <GameDetail v-if="selected" :game="selected" />
      <EmptyState v-else icon="gamepad-2" title="No game selected" />
    </template>
  </ScreenFrame>

  <CustomGameDialog />
</template>
