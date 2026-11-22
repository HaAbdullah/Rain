<script setup lang="ts">
// Life (spec 7.6): habits from the Obsidian daily notes as a month or year heatmap,
// the selected day on the right, today's agenda below it. Strictly read-only.
import { computed, watch } from 'vue'
import { ChevronLeftIcon, ChevronRightIcon, LockIcon } from '@lucide/vue'
import { actionContext } from '@/actions/context'
import type { ActionTarget } from '@/actions/types'
import EmptyState from '@/components/rain/EmptyState.vue'
import ErrorRow from '@/components/rain/ErrorRow.vue'
import ScreenFrame from '@/components/rain/ScreenFrame.vue'
import ViewStrip from '@/components/rain/ViewStrip.vue'
import { Skeleton } from '@/components/ui/skeleton'
import { Tooltip, TooltipContent, TooltipTrigger } from '@/components/ui/tooltip'
import { useNotice } from '@/composables/useNotice'
import { useNow } from '@/composables/useNow'
import { useResource } from '@/composables/useResource'
import { useScreen } from '@/composables/useScreen'
import { useSectionQuery } from '@/composables/useSearch'
import { api } from '@/data'
import type { HabitDay, ID } from '@/data/types'
import { errorMessage } from '@/lib/errors'
import { keepFocus } from '@/lib/focus'
import { matches } from '@/lib/score'
import { formatDayLabel, toISODate } from '@/lib/time'
import { useNav } from '@/nav/useNav'
import { openTodaysNote } from './actions'
import Agenda from './Agenda.vue'
import DayPanel from './DayPanel.vue'
import { addDays, parseISODate } from './dates'
import { doneCount, habitWords, streakAt } from './heat'
import MonthHeatmap from './MonthHeatmap.vue'
import { ALL_HABITS, useDayCursor, type LifeView } from './useDayCursor'
import YearHeatmap from './YearHeatmap.vue'

const VIEWS: readonly LifeView[] = ['month', 'year']
const STRIP_ITEMS = [
  { id: 'month', label: 'Month', icon: 'calendar' },
  { id: 'year', label: 'Year', icon: 'calendar-range' },
]

const nav = useNav()
const notice = useNotice()
const query = useSectionQuery('life')
const now = useNow()
const today = computed(() => toISODate(new Date(now.value)))

const view = computed<LifeView>({
  get: () => VIEWS.find((id) => id === nav.route.value.sub) ?? 'month',
  set: (id) => nav.setSub(id),
})

// ---- data ----

// One range feeds both views and the streaks: last year and this year.
// Both follow `today`, so the screen stays right when it is left open past midnight or New Year.
const thisYear = computed(() => parseISODate(today.value).getFullYear())
const rangeFrom = computed(() => toISODate(new Date(thisYear.value - 1, 0, 1)))
const rangeTo = computed(() => toISODate(new Date(thisYear.value, 11, 31)))

const vault = useResource('vault', () => api.getVaultStatus())
const habits = useResource('habits', () => api.listHabits())
const habitDays = useResource('habitDays:life', () => api.listHabitDays(rangeFrom.value, rangeTo.value))
const events = useResource('events:agenda', () => api.listEvents(today.value, addDays(today.value, 1)))
const settings = useResource('settings', () => api.getSettings())

// The fetchers read the new dates when they run again.
watch(rangeTo, () => void habitDays.reload())
watch(today, () => void events.reload())

const habitList = computed(() => habits.data.value ?? [])
const days = computed(() => new Map((habitDays.data.value ?? []).map((day) => [day.date, day])))
const vaultFound = computed(() => vault.data.value?.connected === true)
const ready = computed(() => vault.data.value && habits.data.value && habitDays.data.value)
const failed = computed(() => vault.error.value ?? habits.error.value ?? habitDays.error.value)

/** Habits whose property no daily note has. */
const missing = computed(
  () =>
    new Set(
      habitList.value
        .filter((habit) => vault.data.value?.missingProperties.includes(habit.property))
        .map((habit) => habit.id),
    ),
)

/** The month rows: the section filter narrows them by habit name. */
const shownHabits = computed(() => habitList.value.filter((habit) => matches(query.value, habit.name)))

function reloadVault() {
  void Promise.all([vault.reload(), habits.reload(), habitDays.reload()])
}

function reloadAll() {
  reloadVault()
  void events.reload()
  void settings.reload()
}

// ---- the cursor ----

const cursor = useDayCursor({
  view,
  today,
  min: rangeFrom,
  rows: computed(() => shownHabits.value.map((habit) => habit.id)),
  chips: computed(() => [ALL_HABITS, ...habitList.value.map((habit) => habit.id)]),
})

// A new filter text selects its first match: a row in Month, the chip in Year.
watch(query, (text) => {
  const first = shownHabits.value[0]
  if (text === '' || !first) return
  cursor.select(cursor.date.value, first.id)
  cursor.selectChip(first.id)
})

const selectedDay = computed<HabitDay>(
  () => days.value.get(cursor.date.value) ?? { date: cursor.date.value, noteExists: false, values: {} },
)

/** The habit the cursor is on: the month row, or the year chip unless it is "All". */
const cursorHabit = computed(() => {
  const id: ID | null = view.value === 'month' ? cursor.row.value : cursor.chip.value
  return habitList.value.find((habit) => habit.id === id)
})

const target = computed<ActionTarget | null>(() =>
  ready.value && vaultFound.value ? { type: 'day', id: cursor.date.value, item: selectedDay.value } : null,
)

// Without a grid to move over, Left/Right belong to the search caret.
const hasGrid = computed(
  () => target.value !== null && habitList.value.length > 0 && (view.value === 'year' || shownHabits.value.length > 0),
)

useScreen({
  target,
  move: (dir) => hasGrid.value && cursor.move(dir),
  page: cursor.page,
  group: cursor.group,
  subView: { ids: VIEWS, current: view },
})

/** True until both the events and the connection setting are known, so no wrong state is painted. */
const agendaLoading = computed(
  () =>
    (events.data.value === undefined && events.error.value === null) ||
    (settings.data.value === undefined && settings.error.value === null),
)

// ---- text ----

const readout = computed(() => {
  const label = formatDayLabel(cursor.date.value)
  if (!selectedDay.value.noteExists) return `${label}, no note`
  const habit = cursorHabit.value
  if (!habit) {
    const { done, counted } = doneCount(
      habitList.value.filter((candidate) => !missing.value.has(candidate.id)),
      selectedDay.value,
    )
    return `${label}, ${done} of ${counted} habits done`
  }
  const streak = missing.value.has(habit.id) ? 0 : streakAt(habit, days.value, cursor.date.value, today.value)
  return `${label}, ${habitWords(habit, selectedDay.value)}${streak > 0 ? `, streak ${streak}` : ''}`
})

const periodLabel = computed(() => {
  const date = parseISODate(cursor.date.value)
  return view.value === 'year'
    ? String(date.getFullYear())
    : new Intl.DateTimeFormat('en-GB', { month: 'long', year: 'numeric' }).format(date)
})

// ---- actions ----

function pickView(id: string) {
  const known = VIEWS.find((candidate) => candidate === id)
  if (known) view.value = known
}

async function openToday() {
  try {
    await openTodaysNote(actionContext())
  } catch (reason) {
    notice.error(errorMessage(reason, "Could not open today's note"))
  }
}

function openSettings() {
  nav.go('settings')
}
</script>

<template>
  <ScreenFrame pane="md">
    <template #strip>
      <ViewStrip :items="STRIP_ITEMS" :model-value="view" @update:model-value="pickView">
        <template #end>
          <button
            type="button"
            tabindex="-1"
            aria-label="Previous"
            class="flex size-7 items-center justify-center rounded-md text-muted-foreground transition-colors hover:bg-hover hover:text-foreground"
            @click="cursor.page(-1)"
          >
            <ChevronLeftIcon :size="16" :stroke-width="1.75" />
          </button>
          <span class="min-w-28 text-center text-13 text-foreground">{{ periodLabel }}</span>
          <button
            type="button"
            tabindex="-1"
            aria-label="Next"
            class="flex size-7 items-center justify-center rounded-md text-muted-foreground transition-colors hover:bg-hover hover:text-foreground"
            @click="cursor.page(1)"
          >
            <ChevronRightIcon :size="16" :stroke-width="1.75" />
          </button>
          <button
            type="button"
            tabindex="-1"
            class="h-7 rounded-md px-2 text-13 text-muted-foreground transition-colors hover:bg-hover hover:text-foreground"
            @click="cursor.goToday()"
          >
            Today
          </button>
          <Tooltip>
            <TooltipTrigger as-child>
              <LockIcon :size="16" :stroke-width="1.75" class="mx-2 text-text-3" aria-label="Read-only" />
            </TooltipTrigger>
            <TooltipContent>Rain reads your vault and never writes to it</TooltipContent>
          </Tooltip>
        </template>
      </ViewStrip>
    </template>

    <div v-if="failed" class="p-4" @mousedown="keepFocus">
      <ErrorRow message="Vault unavailable" @retry="reloadVault" />
    </div>

    <div v-else-if="!ready" class="flex flex-col gap-3 p-4">
      <template v-if="vault.showSkeleton.value || habits.showSkeleton.value || habitDays.showSkeleton.value">
        <Skeleton v-if="view === 'year'" class="h-7 w-2/3" />
        <Skeleton v-if="view === 'year'" class="h-40" />
        <template v-else>
          <Skeleton v-for="row in 5" :key="row" class="h-5" />
        </template>
      </template>
    </div>

    <EmptyState
      v-else-if="!vaultFound"
      icon="calendar-check"
      :title="vault.data.value?.path ? 'Vault not found' : 'Choose your vault'"
      :hint="
        vault.data.value?.path
          ? `Nothing at ${vault.data.value.path}. Check the path in Settings.`
          : 'Point Rain at your Obsidian vault to see your habits.'
      "
      action-label="Open settings"
      @action="openSettings"
    />

    <EmptyState
      v-else-if="habitList.length === 0"
      icon="calendar-check"
      title="No habits found"
      hint="Add habit properties to your daily notes, then read the vault again."
      action-label="Re-read vault"
      action-keys="Ctrl+KeyR"
      @action="reloadAll"
    />

    <EmptyState v-else-if="view === 'month' && shownHabits.length === 0" icon="search-x" title="No habit matches" />

    <MonthHeatmap
      v-else-if="view === 'month'"
      :habits="shownHabits"
      :days="days"
      :date="cursor.date.value"
      :today="today"
      :row="cursor.row.value"
      :missing="missing"
      @select="cursor.select"
    />

    <YearHeatmap
      v-else
      :habits="habitList"
      :days="days"
      :date="cursor.date.value"
      :today="today"
      :chip="cursor.chip.value"
      :missing="missing"
      @select="cursor.select"
      @update:chip="cursor.selectChip"
    />

    <template #pane>
      <DayPanel
        :date="cursor.date.value"
        :readout="readout"
        :habits="habitList"
        :day="selectedDay"
        :vault-found="vaultFound"
        @open-today="openToday"
      >
        <Agenda
          :events="events.data.value ?? []"
          :now="now"
          :today="today"
          :connected="settings.data.value?.calendarConnected ?? true"
          :loading="agendaLoading"
          :show-skeleton="events.showSkeleton.value"
          :failed="events.error.value !== null"
          @connect="openSettings"
          @retry="events.reload"
        />
      </DayPanel>
    </template>
  </ScreenFrame>
</template>
