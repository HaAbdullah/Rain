<script setup lang="ts">
// The year view: habit chips above a grid of week columns and weekday rows.
import { computed } from 'vue'
import RIcon from '@/components/rain/RIcon.vue'
import { ToggleGroup, ToggleGroupItem } from '@/components/ui/toggle-group'
import type { Habit, HabitDay, ID, ISODate } from '@/data/types'
import { keepFocus } from '@/lib/focus'
import { toISODate } from '@/lib/time'
import { daysBetween, parseISODate, weekdayRow } from './dates'
import HeatCell from './HeatCell.vue'
import { allState, cellState, type CellState } from './heat'
import { ALL_HABITS } from './useDayCursor'

const props = defineProps<{
  habits: Habit[]
  days: Map<ISODate, HabitDay>
  /** The selected day; its year is the one shown. */
  date: ISODate
  today: ISODate
  /** The chip that is on: "all" or a habit id. */
  chip: ID
  /** Habits whose property no note has; "All" leaves them out. */
  missing: ReadonlySet<ID>
}>()

const emit = defineEmits<{ select: [date: ISODate]; 'update:chip': [id: ID] }>()

const WEEKDAY_LABELS = ['M', '', 'W', '', 'F', '', '']
const MONTH_NAMES = Array.from({ length: 12 }, (_, i) =>
  new Intl.DateTimeFormat('en-GB', { month: 'short' }).format(new Date(2000, i, 1)),
)

const year = computed(() => parseISODate(props.date).getFullYear())
const firstDay = computed(() => toISODate(new Date(year.value, 0, 1)))
const yearLength = computed(() => daysBetween(firstDay.value, toISODate(new Date(year.value, 11, 31))) + 1)
/** Grid slots before the 1st of January, so that the first week lines up under Monday. */
const lead = computed(() => weekdayRow(firstDay.value))

interface Cell {
  date: ISODate
  column: number
  row: number
  state: CellState
}

function columnOf(date: ISODate): number {
  return Math.floor((daysBetween(firstDay.value, date) + lead.value) / 7)
}

const cells = computed<Cell[]>(() => {
  const habit = props.habits.find((candidate) => candidate.id === props.chip)
  const counted = props.habits.filter((candidate) => !props.missing.has(candidate.id))
  // Future days are not drawn.
  const drawn = Math.min(yearLength.value, daysBetween(firstDay.value, props.today) + 1)
  return Array.from({ length: Math.max(0, drawn) }, (_, i) => {
    const date = toISODate(new Date(year.value, 0, i + 1))
    const day = props.days.get(date)
    return {
      date,
      column: columnOf(date),
      row: weekdayRow(date),
      state: habit ? cellState(habit, day) : allState(counted, day),
    }
  })
})

const weekCount = computed(() => Math.ceil((lead.value + yearLength.value) / 7))

const monthLabels = computed(() =>
  MONTH_NAMES.flatMap((name, month) => {
    const date = toISODate(new Date(year.value, month, 1))
    return date <= props.today ? [{ name, column: columnOf(date) }] : []
  }),
)

// Cells shrink with the window; 12 px is their size when there is room.
const columns = computed(() => `auto repeat(${weekCount.value}, minmax(0, 12px))`)

function pickChip(value: unknown) {
  // A second click on the lit chip would switch it off; one chip is always on.
  if (typeof value === 'string' && value !== '') emit('update:chip', value)
}
</script>

<template>
  <div class="p-4" @mousedown="keepFocus">
    <ToggleGroup
      type="single"
      :model-value="chip"
      :spacing="1"
      class="mb-4 w-full flex-wrap"
      @update:model-value="pickChip"
    >
      <ToggleGroupItem :value="ALL_HABITS" size="sm" tabindex="-1" class="gap-1.5 text-xs">All</ToggleGroupItem>
      <ToggleGroupItem
        v-for="habit in habits"
        :key="habit.id"
        :value="habit.id"
        size="sm"
        tabindex="-1"
        class="gap-1.5 text-xs"
      >
        <RIcon :icon="habit.icon" :size="12" />
        {{ habit.name }}
      </ToggleGroupItem>
    </ToggleGroup>

    <div class="grid gap-[3px]" :style="{ gridTemplateColumns: columns }">
      <span
        v-for="label in monthLabels"
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
        v-for="cell in cells"
        :key="cell.date"
        :state="cell.state"
        :today="cell.date === today"
        :selected="cell.date === date"
        :style="{ gridColumnStart: cell.column + 2, gridRowStart: cell.row + 2 }"
        @click="emit('select', cell.date)"
      />
    </div>
  </div>
</template>
