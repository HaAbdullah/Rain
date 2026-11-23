<script setup lang="ts">
// The year grid of ONE habit: 53 week columns by 7 weekday rows, month names above, M W F on
// the left. Done is solid, a day with a note but no tick is dim, a day with no note is hollow.
// Days older than Rain will change are drawn faint; days before the history window are hollow.
import { computed } from 'vue'
import type { HabitCell, ISODate } from '@/data/types'
import HeatCell from './HeatCell.vue'
import { yearGrid } from './yearGrid'

const props = defineProps<{
  year: number
  today: ISODate
  /** The day the cursor is on. */
  selected: ISODate
  /** How a day looks: done, miss (a note without the tick) or none (no note). */
  stateOf: (date: ISODate) => HabitCell
  /** The oldest day Rain will tick. Older days are drawn at half strength. */
  tickableFrom: ISODate
  /** The oldest day of the history window. Older days are always hollow. */
  windowFrom: ISODate
}>()

const emit = defineEmits<{ select: [date: ISODate] }>()

const WEEKDAY_LABELS = ['M', '', 'W', '', 'F', '', '']

const grid = computed(() => yearGrid(props.year, props.today))

// Cells shrink with the window; 12 px is their size when there is room.
const columns = computed(() => `auto repeat(${grid.value.weekCount}, minmax(0, 12px))`)
</script>

<template>
  <div class="grid gap-[3px]" :style="{ gridTemplateColumns: columns }">
    <span
      v-for="label in grid.labels"
      :key="label.name"
      class="row-start-1 pb-1 text-xs whitespace-nowrap text-text-3"
      :style="{ gridColumnStart: label.column + 2 }"
    >
      {{ label.name }}
    </span>

    <span
      v-for="(label, row) in WEEKDAY_LABELS"
      :key="row"
      class="col-start-1 self-center pr-2 text-xs leading-none text-text-3"
      :style="{ gridRowStart: row + 2 }"
    >
      {{ label }}
    </span>

    <HeatCell
      v-for="cell in grid.cells"
      :key="cell.date"
      :state="cell.date < windowFrom ? 'none' : stateOf(cell.date)"
      :today="cell.date === today"
      :selected="cell.date === selected"
      :class="{ 'opacity-50': cell.date < tickableFrom }"
      :style="{ gridColumnStart: cell.column + 2, gridRowStart: cell.row + 2 }"
      @click="emit('select', cell.date)"
    />
  </div>
</template>
