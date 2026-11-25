<script setup lang="ts">
// The aggregate heatmap at the bottom of Life: every habit in one picture, like a GitHub
// contribution graph. One column per week, newest on the right, as many weeks as fit up to a
// year. A cell shows how much of that day's habits were done. It only draws; it does nothing
// when clicked.
import { useElementSize } from '@vueuse/core'
import { computed, ref } from 'vue'
import type { HabitDay, HabitSummary, ISODate } from '@/data/types'
import { formatDayLabel } from '@/lib/time'
import { heatModel, HEAT_MAX_WEEKS, weeksThatFit, type HeatStep } from './allHabitsHeat'

const props = defineProps<{
  habits: readonly HabitSummary[]
  days: readonly HabitDay[]
  today: ISODate
}>()

const CELL = 10
const GAP = 3
const STEP_CLASS: Record<HeatStep, string> = {
  0: 'bg-heat-0',
  1: 'bg-heat-1',
  2: 'bg-heat-2',
  3: 'bg-heat-3',
  4: 'bg-heat-4',
}
const STEPS: HeatStep[] = [0, 1, 2, 3, 4]

// The width the grid may use. Before the first measure (and where nothing measures) a full year.
const box = ref<HTMLElement | null>(null)
const { width } = useElementSize(box)
const weeks = computed(() => (width.value > 0 ? weeksThatFit(width.value, CELL, GAP) : HEAT_MAX_WEEKS))

const model = computed(() => heatModel(props.habits, props.days, props.today, weeks.value))
const columns = computed(() => `repeat(${model.value.weekCount}, ${CELL}px)`)

function tip(date: ISODate, done: number, total: number): string {
  return `${formatDayLabel(date)}: ${done} of ${total} done`
}
</script>

<template>
  <section class="flex shrink-0 flex-col gap-1.5 border-t border-border px-4 py-2" aria-label="All habits">
    <div class="flex items-baseline gap-3">
      <h2 class="text-13 font-medium text-foreground">All habits</h2>
      <span class="text-xs text-text-3" data-testid="full-days">
        {{ model.fullDays }} {{ model.fullDays === 1 ? 'day' : 'days' }} with everything done
      </span>
      <div class="ml-auto flex items-center gap-1 text-xs text-text-3" aria-hidden="true">
        <span>Less</span>
        <span
          v-for="step in STEPS"
          :key="step"
          class="rounded-cell"
          :class="STEP_CLASS[step]"
          :style="{ width: `${CELL}px`, height: `${CELL}px` }"
        />
        <span>More</span>
      </div>
    </div>

    <div ref="box" class="min-w-0 overflow-hidden">
      <div class="grid w-max" :style="{ gridTemplateColumns: columns, gap: `${GAP}px` }">
        <span
          v-for="label in model.labels"
          :key="`${label.column}-${label.name}`"
          class="row-start-1 text-xs leading-none whitespace-nowrap text-text-3"
          :style="{ gridColumnStart: label.column + 1 }"
        >
          {{ label.name }}
        </span>
        <div
          v-for="day in model.days"
          :key="day.date"
          class="rounded-cell"
          :class="[STEP_CLASS[day.step], day.date === today ? 'outline-1 outline-foreground' : '']"
          :data-step="day.step"
          :title="tip(day.date, day.done, day.total)"
          :style="{
            gridColumnStart: day.column + 1,
            gridRowStart: day.row + 2,
            width: `${CELL}px`,
            height: `${CELL}px`,
          }"
        />
      </div>
    </div>
  </section>
</template>
