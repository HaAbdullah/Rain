<script setup lang="ts">
// The tile grid of the Hardware screen (spec 7.7): one big tile for the disks, the keyboard and
// the mouse. The backend reads the devices only when this component is shown and on Ctrl+R, so
// the screen costs nothing while it is not open.
import { computed } from 'vue'
import { runPrimary } from '@/actions/context'
import type { ActionTarget } from '@/actions/types'
import EmptyState from '@/components/rain/EmptyState.vue'
import ErrorRow from '@/components/rain/ErrorRow.vue'
import ItemGrid from '@/components/rain/ItemGrid.vue'
import ScreenFrame from '@/components/rain/ScreenFrame.vue'
import { useActionPanel } from '@/composables/useActionPanel'
import { useResource } from '@/composables/useResource'
import { useScreen } from '@/composables/useScreen'
import { useSectionQuery } from '@/composables/useSearch'
import { useSelection, type SelGroup } from '@/composables/useSelection'
import { api } from '@/data'
import type { HardwareTile } from '@/data/types'
import { matches } from '@/lib/score'
import { HARDWARE_RESOURCE } from './actions'
import HardwareCard from './HardwareCard.vue'
import { placeholderTiles } from './model'

/** Tiles per row. The sketch is one row of three wide tiles; a fourth device starts a new row. */
const COLUMNS = 3

const query = useSectionQuery('hardware')
const panel = useActionPanel()
const hardware = useResource(HARDWARE_RESOURCE, () => api.listHardware())

/** Name and icon from the first moment; the answer replaces them. */
const tiles = computed<HardwareTile[]>(() => {
  if (hardware.data.value) return hardware.data.value
  // No answer and an error: the tiles say so, instead of "not found".
  return placeholderTiles().map((tile) => (hardware.error.value ? { ...tile, problem: 'Not read' } : tile))
})
// The lines are bars until the first answer. A refresh keeps the old lines on screen.
const loading = computed(() => hardware.data.value === undefined && hardware.error.value === null)

const visible = computed(() =>
  tiles.value.filter((tile) => matches(query.value, tile.name, [tile.product ?? '', tile.kind])),
)
const groups = computed<SelGroup<HardwareTile>[]>(() => [
  { id: 'hardware', items: visible.value, columns: COLUMNS },
])

const { selectedId, selected, move, page, select } = useSelection(groups, {
  resetOn: query,
  persistKey: 'hardware',
})

function targetOf(tile: HardwareTile): ActionTarget {
  return { type: 'hardware', id: tile.id, item: tile }
}

useScreen({
  target: computed(() => (selected.value ? targetOf(selected.value) : null)),
  move,
  page,
})

function openPanel(tile: HardwareTile, event: MouseEvent) {
  select(tile.id)
  panel.open(targetOf(tile), event)
}
</script>

<template>
  <ScreenFrame>
    <div v-if="hardware.error.value" class="px-4 pt-4">
      <ErrorRow message="Hardware unavailable" :detail="hardware.error.value.message" @retry="hardware.reload()" />
    </div>

    <EmptyState v-if="visible.length === 0" icon="search-x" title="No match" />
    <ItemGrid
      v-else
      :groups="groups"
      :selected-id="selectedId"
      :columns="COLUMNS"
      :gap="16"
      @update:selected-id="select"
      @activate="runPrimary(targetOf($event))"
      @context="openPanel"
    >
      <template #tile="{ item, selected: isSelected }">
        <HardwareCard :tile="item" :selected="isSelected" :loading="loading" />
      </template>
    </ItemGrid>
  </ScreenFrame>
</template>
