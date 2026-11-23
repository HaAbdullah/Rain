<script setup lang="ts">
// The small history strip of a list row: a daily habit shows its last 70 days, a weekly one its
// last 12 weeks, a monthly one its last 12 months. Oldest on the left; the last cell is the
// current day, week or month and has the outline. No tooltips: the full view has the detail.
import { computed } from 'vue'
import type { Cadence, HabitCell } from '@/data/types'
import HeatCell from './HeatCell.vue'

const props = defineProps<{
  cells: HabitCell[]
  cadence: Cadence
}>()

// Cells shrink with the window; these are their sizes when there is room.
const SIZE = { daily: { cell: 10, gap: 2 }, weekly: { cell: 14, gap: 3 }, monthly: { cell: 14, gap: 3 } }

const style = computed(() => {
  const { cell, gap } = SIZE[props.cadence]
  return {
    gridTemplateColumns: `repeat(${props.cells.length}, minmax(0, ${cell}px))`,
    gap: `${gap}px`,
  }
})

const label = computed(() =>
  props.cadence === 'daily' ? 'Last 10 weeks' : props.cadence === 'weekly' ? 'Last 12 weeks' : 'Last 12 months',
)
</script>

<template>
  <div class="grid min-w-0 shrink" role="img" :aria-label="label" :style="style">
    <HeatCell
      v-for="(cell, index) in cells"
      :key="index"
      :state="cell"
      :today="index === cells.length - 1"
      :selected="false"
    />
  </div>
</template>
