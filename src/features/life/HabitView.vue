<script setup lang="ts">
// The full view of one habit (a sub-view of Life, not a route): its streaks, rate and totals,
// and the year grid with every day. Opened with Ctrl+Enter or a click on the row; Esc or the
// back button returns to the list.
//
// The numbers come from the backend (`getHabitStats`); this component only lays them out.
import { ArrowLeftIcon, ChevronLeftIcon, ChevronRightIcon, PencilIcon } from '@lucide/vue'
import { computed } from 'vue'
import ErrorRow from '@/components/rain/ErrorRow.vue'
import KeyHint from '@/components/rain/KeyHint.vue'
import RIcon from '@/components/rain/RIcon.vue'
import { Checkbox } from '@/components/ui/checkbox'
import { Skeleton } from '@/components/ui/skeleton'
import { Tooltip, TooltipContent, TooltipTrigger } from '@/components/ui/tooltip'
import type { Cadence, HabitSummary } from '@/data/types'
import { keepFocus } from '@/lib/focus'
import HabitYear from './HabitYear.vue'
import type { OpenHabit } from './useOpenHabit'

const props = defineProps<{
  summary: HabitSummary
  open: OpenHabit
  today: string
  /** False when Rain may not tick (a development build with a real vault). */
  writable: boolean
}>()

const emit = defineEmits<{ back: []; edit: []; 'tick-today': [] }>()

const UNITS: Record<Cadence, [string, string]> = {
  daily: ['day', 'days'],
  weekly: ['week', 'weeks'],
  monthly: ['month', 'months'],
}

/** "1 week", "5 days": the streaks count in the unit of the cadence. */
function inUnit(count: number): string {
  const [one, many] = UNITS[props.summary.habit.cadence]
  return `${count} ${count === 1 ? one : many}`
}

const cadenceText = computed(() => {
  const { cadence, times } = props.summary.habit
  if (cadence === 'daily') return 'Daily'
  const label = cadence === 'weekly' ? 'Weekly' : 'Monthly'
  return `${label}, ${times} ${times === 1 ? 'time' : 'times'}`
})

/** The figures under the header, as label and text. */
const figures = computed(() => {
  const stats = props.open.stats.value
  if (!stats) return []
  return [
    { label: 'Streak', value: inUnit(stats.streakCurrent) },
    { label: 'Best', value: inUnit(stats.streakBest) },
    { label: 'Rate', value: stats.ratePercent === undefined ? '-' : `${stats.ratePercent}%` },
    { label: 'This week', value: String(stats.doneThisWeek) },
    { label: 'This month', value: String(stats.doneThisMonth) },
    { label: 'This year', value: String(stats.doneThisYear) },
  ]
})

const iconButton =
  'flex size-8 items-center justify-center rounded-md text-muted-foreground transition-colors hover:bg-hover hover:text-foreground'
</script>

<template>
  <div class="flex flex-col gap-4 p-4" @mousedown="keepFocus">
    <!-- header: back, the habit, today's checkbox, edit -->
    <div class="flex items-center gap-3">
      <Tooltip>
        <TooltipTrigger as-child>
          <button type="button" tabindex="-1" aria-label="Back" :class="iconButton" @click="emit('back')">
            <ArrowLeftIcon :size="16" :stroke-width="1.75" />
          </button>
        </TooltipTrigger>
        <TooltipContent>
          <span>Back</span>
          <KeyHint keys="Escape" />
        </TooltipContent>
      </Tooltip>

      <RIcon :icon="summary.habit.icon" :size="20" :label="summary.habit.name" />
      <div class="flex min-w-0 flex-1 items-baseline gap-3">
        <h2 class="selectable min-w-0 truncate text-base font-semibold text-foreground">
          {{ summary.habit.name }}
        </h2>
        <span class="shrink-0 text-13 text-muted-foreground">{{ cadenceText }}</span>
      </div>

      <label class="flex items-center gap-2 text-13 text-muted-foreground">
        today
        <Checkbox
          :model-value="summary.todayDone"
          :disabled="!writable"
          tabindex="-1"
          :aria-label="`${summary.habit.name}, today`"
          @update:model-value="emit('tick-today')"
        />
      </label>

      <Tooltip>
        <TooltipTrigger as-child>
          <button type="button" tabindex="-1" aria-label="Edit habit" :class="iconButton" @click="emit('edit')">
            <PencilIcon :size="16" :stroke-width="1.75" />
          </button>
        </TooltipTrigger>
        <TooltipContent>Edit habit</TooltipContent>
      </Tooltip>
    </div>

    <div v-if="open.failed.value" class="-mx-2">
      <ErrorRow message="Habit unavailable" @retry="open.reload()" />
    </div>

    <div v-else-if="!open.ready.value" class="flex flex-col gap-4" aria-hidden="true">
      <Skeleton class="h-12 w-2/3" />
      <Skeleton class="h-40" />
    </div>

    <template v-else>
      <dl class="flex flex-wrap gap-x-8 gap-y-3">
        <div v-for="figure in figures" :key="figure.label">
          <dt class="text-xs text-text-3">{{ figure.label }}</dt>
          <dd class="text-xl font-semibold text-foreground">{{ figure.value }}</dd>
        </div>
      </dl>

      <div class="flex flex-col gap-2">
        <div class="flex items-center gap-1">
          <button type="button" tabindex="-1" aria-label="Previous year" :class="iconButton" @click="open.page(-1)">
            <ChevronLeftIcon :size="16" :stroke-width="1.75" />
          </button>
          <span class="min-w-12 text-center text-13 text-foreground">{{ open.year.value }}</span>
          <button type="button" tabindex="-1" aria-label="Next year" :class="iconButton" @click="open.page(1)">
            <ChevronRightIcon :size="16" :stroke-width="1.75" />
          </button>
        </div>

        <HabitYear
          :year="open.year.value"
          :today="today"
          :selected="open.date.value"
          :state-of="open.cellState"
          :tickable-from="open.tickableFrom.value"
          :window-from="open.windowFrom.value"
          @select="open.select"
        />
      </div>

      <p class="selectable text-13 text-muted-foreground">{{ open.readout.value }}</p>
    </template>
  </div>
</template>
