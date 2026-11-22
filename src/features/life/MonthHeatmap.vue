<script setup lang="ts">
// The month view: one row per habit, one square per day, then streak and completion.
import { computed } from 'vue'
import { FlameIcon, TriangleAlertIcon } from '@lucide/vue'
import RIcon from '@/components/rain/RIcon.vue'
import { Tooltip, TooltipContent, TooltipTrigger } from '@/components/ui/tooltip'
import type { Habit, HabitDay, ID, ISODate } from '@/data/types'
import { keepFocus } from '@/lib/focus'
import { toISODate } from '@/lib/time'
import { daysInMonth, isWeekend, parseISODate } from './dates'
import HeatCell from './HeatCell.vue'
import { cellState, completionPercent, streakAt } from './heat'

const props = defineProps<{
  habits: Habit[]
  days: Map<ISODate, HabitDay>
  /** The selected day; its month is the one shown. */
  date: ISODate
  today: ISODate
  /** The habit row of the cursor. */
  row: ID | null
  /** Habits whose property no note has. */
  missing: ReadonlySet<ID>
}>()

const emit = defineEmits<{ select: [date: ISODate, habit: ID] }>()

const monthDates = computed<ISODate[]>(() => {
  const shown = parseISODate(props.date)
  const year = shown.getFullYear()
  const month = shown.getMonth()
  return Array.from({ length: daysInMonth(year, month) }, (_, i) => toISODate(new Date(year, month, i + 1)))
})

/** Days that are drawn: future days are not. ISO dates compare correctly as text. */
const pastDates = computed(() => monthDates.value.filter((date) => date <= props.today))

// Cells shrink with the window; 20 px is their size when there is room.
const columns = computed(() => `132px repeat(${monthDates.value.length}, minmax(0, 20px)) 40px 40px`)

interface Row {
  habit: Habit
  streak: number | null
  completion: number | null
}

const rows = computed<Row[]>(() => {
  const lastDrawn = pastDates.value[pastDates.value.length - 1]
  const dayList = pastDates.value.flatMap((date) => props.days.get(date) ?? [])
  return props.habits.map((habit) => {
    const tracked = !props.missing.has(habit.id)
    return {
      habit,
      streak: tracked ? streakAt(habit, props.days, lastDrawn, props.today) : null,
      completion: tracked ? completionPercent(habit, dayList) : null,
    }
  })
})

function dash(value: number | null, suffix = ''): string {
  return value === null ? '–' : `${value}${suffix}`
}
</script>

<template>
  <!-- The container query decides how many day numbers fit above the cells. -->
  <div class="@container p-4" @mousedown="keepFocus">
    <div class="grid items-center gap-x-[3px] gap-y-1.5" :style="{ gridTemplateColumns: columns }">
      <span />
      <span
        v-for="(date, i) in monthDates"
        :key="date"
        class="text-center text-xs"
        :class="[
          isWeekend(date) ? 'text-muted-foreground' : 'text-text-3',
          (i + 1) % 2 === 0 ? 'invisible @min-[800px]:visible' : '',
        ]"
      >
        {{ i + 1 }}
      </span>
      <FlameIcon :size="12" :stroke-width="1.75" class="justify-self-end text-text-3" aria-label="Streak" />
      <span class="text-right text-xs text-text-3">Done</span>

      <template v-for="{ habit, streak, completion } in rows" :key="habit.id">
        <div class="flex min-w-0 items-center gap-2">
          <RIcon :icon="habit.icon" :size="16" />
          <span
            class="min-w-0 truncate text-13"
            :class="habit.id === row ? 'text-foreground' : 'text-muted-foreground'"
          >
            {{ habit.name }}
          </span>
          <Tooltip v-if="missing.has(habit.id)">
            <TooltipTrigger as-child>
              <TriangleAlertIcon
                :size="12"
                :stroke-width="1.75"
                class="shrink-0 text-status-waiting"
                aria-label="Property not found"
              />
            </TooltipTrigger>
            <TooltipContent>Property '{{ habit.property }}' not found in notes</TooltipContent>
          </Tooltip>
        </div>

        <template v-for="date in monthDates" :key="date">
          <HeatCell
            v-if="date <= today"
            :state="cellState(habit, days.get(date))"
            :today="date === props.today"
            :selected="habit.id === row && date === props.date"
            @click="emit('select', date, habit.id)"
          />
          <span v-else />
        </template>

        <span class="text-right text-13 text-muted-foreground">{{ dash(streak) }}</span>
        <span class="text-right text-13 text-muted-foreground">{{ dash(completion, '%') }}</span>
      </template>
    </div>
  </div>
</template>
