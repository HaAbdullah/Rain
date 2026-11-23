<script setup lang="ts">
// One day, week or month of a habit in a heat strip or in the year grid. It only draws; the
// parent decides what a click does.
import type { HabitCell } from '@/data/types'

defineProps<{
  state: HabitCell
  today: boolean
  selected: boolean
}>()

// `none` is hollow: there is no daily note that day. A miss has a note but no tick.
const FILL: Record<HabitCell, string> = {
  none: 'border border-selected',
  miss: 'bg-heat-0',
  partial: 'bg-heat-1',
  done: 'bg-heat-3',
}
</script>

<template>
  <div
    class="aspect-square w-full rounded-cell"
    :class="[
      FILL[state],
      selected ? 'outline-2 outline-ring' : today ? 'outline-1 outline-foreground' : '',
    ]"
  />
</template>
