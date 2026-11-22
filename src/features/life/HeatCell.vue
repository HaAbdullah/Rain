<script setup lang="ts">
// One day of a habit in the heatmaps. It only draws; the grid decides what a click does.
import type { CellState } from './heat'

defineProps<{
  state: CellState
  today: boolean
  selected: boolean
}>()

const FILL: Record<CellState, string> = {
  'none': 'border border-selected',
  'miss': 'bg-heat-0',
  'skip': 'bg-heat-0',
  'heat-1': 'bg-heat-1',
  'heat-2': 'bg-heat-2',
  'heat-3': 'bg-heat-3',
  'heat-4': 'bg-heat-4',
}
</script>

<template>
  <div
    class="flex aspect-square w-full items-center justify-center rounded-cell"
    :class="[
      FILL[state],
      selected ? 'outline-2 outline-ring' : today ? 'outline-1 outline-foreground' : '',
    ]"
  >
    <span v-if="state === 'skip'" class="h-0.5 w-1.5 bg-text-3" />
  </div>
</template>
