<script setup lang="ts">
// The read-only side of Home: today's date, the habit tiles, the next calendar
// events and the "Today's note" button. It is not part of the keyboard cursor.
import { MinusIcon, NotebookPenIcon } from '@lucide/vue'
import { computed } from 'vue'
import { commands } from '@/actions/commands'
import { runPrimary } from '@/actions/context'
import ErrorRow from '@/components/rain/ErrorRow.vue'
import KeyHint from '@/components/rain/KeyHint.vue'
import RIcon from '@/components/rain/RIcon.vue'
import { Button } from '@/components/ui/button'
import { Separator } from '@/components/ui/separator'
import { Skeleton } from '@/components/ui/skeleton'
import { Tooltip, TooltipContent, TooltipTrigger } from '@/components/ui/tooltip'
import { useNow } from '@/composables/useNow'
import { useResource } from '@/composables/useResource'
import { api } from '@/data'
import type { CalendarEvent, Habit, HabitValue } from '@/data/types'
import { keepFocus } from '@/lib/focus'
import { formatClock, formatDayLabel, toISODate } from '@/lib/time'
import { useNav } from '@/nav/useNav'

const MAX_HABITS = 7
const MAX_EVENTS = 4
const DAY_MS = 24 * 60 * 60 * 1000

const nav = useNav()
const now = useNow()

// The range reaches into tomorrow, so the evening still has "next" events.
const from = toISODate(new Date())
const to = toISODate(new Date(Date.now() + DAY_MS))

const habits = useResource('habits', () => api.listHabits())
const settings = useResource('settings', () => api.getSettings())
const events = useResource(`events:${from}:${to}`, () => api.listEvents(from, to))
const days = useResource(`habitDays:${from}:${from}`, () => api.listHabitDays(from, from))

const shownHabits = computed(() => (habits.data.value ?? []).slice(0, MAX_HABITS))
const todayValues = computed(() => days.data.value?.find((day) => day.date === from)?.values ?? {})

const habitsFailed = computed(() => habits.error.value !== null || days.error.value !== null)
const habitsLoading = computed(() => habits.showSkeleton.value || days.showSkeleton.value)

function reloadHabits() {
  void Promise.all([habits.reload(), days.reload()])
}

const nextEvents = computed(() =>
  (events.data.value ?? [])
    .filter((event) => Date.parse(event.end) > now.value)
    .sort((a, b) => Date.parse(a.start) - Date.parse(b.start))
    .slice(0, MAX_EVENTS),
)

function isDone(habit: Habit, value: HabitValue | undefined): boolean {
  if (typeof value === 'number') return value >= (habit.target ?? 1)
  return value === true
}

function valueText(habit: Habit, value: HabitValue | undefined): string {
  if (value === 'skip') return 'skipped'
  if (typeof value === 'number') return `${value}${habit.unit ? ` ${habit.unit}` : ''}`
  return isDone(habit, value) ? 'done' : 'not yet'
}

function isCurrent(event: CalendarEvent): boolean {
  return Date.parse(event.start) <= now.value
}

const todayNote = commands.find((command) => command.id === 'today-note')

function openTodayNote() {
  if (todayNote) runPrimary({ type: 'command', id: todayNote.id, item: todayNote })
}
</script>

<template>
  <div class="flex h-full flex-col gap-4 p-4">
    <div class="flex items-baseline justify-between">
      <h2 class="text-base font-semibold text-foreground">Today</h2>
      <span class="text-xs text-muted-foreground">{{ formatDayLabel(from) }}</span>
    </div>

    <Separator />

    <div v-if="habitsFailed" class="-mx-2">
      <ErrorRow message="Habits unavailable" @retry="reloadHabits()" />
    </div>
    <div v-else-if="habitsLoading" class="flex gap-2" aria-hidden="true">
      <Skeleton v-for="n in MAX_HABITS" :key="n" class="size-7 rounded-md" />
    </div>
    <p v-else-if="habits.data.value && shownHabits.length === 0" class="text-13 text-muted-foreground">
      <button
        type="button"
        tabindex="-1"
        class="underline-offset-2 transition-colors hover:text-foreground hover:underline"
        @mousedown="keepFocus"
        @click="nav.go('settings')"
      >
        No habits yet. Check the vault in Settings
      </button>
    </p>
    <div v-else class="flex flex-wrap gap-2">
      <Tooltip v-for="habit in shownHabits" :key="habit.id">
        <TooltipTrigger as-child>
          <button
            type="button"
            tabindex="-1"
            class="flex size-7 items-center justify-center rounded-md transition-colors"
            :class="
              isDone(habit, todayValues[habit.id])
                ? 'bg-heat-3 text-foreground'
                : 'bg-surface-2 text-muted-foreground'
            "
            :aria-label="`${habit.name}: ${valueText(habit, todayValues[habit.id])}`"
            @mousedown="keepFocus"
            @click="nav.go('life')"
          >
            <MinusIcon v-if="todayValues[habit.id] === 'skip'" :size="16" :stroke-width="1.75" aria-hidden="true" />
            <RIcon v-else :icon="habit.icon" :size="16" />
          </button>
        </TooltipTrigger>
        <TooltipContent>{{ habit.name }}: {{ valueText(habit, todayValues[habit.id]) }}</TooltipContent>
      </Tooltip>
    </div>

    <Separator />

    <div class="flex min-h-0 flex-col gap-1">
      <div v-if="events.error.value" class="-mx-2">
        <ErrorRow message="Calendar unavailable" @retry="events.reload()" />
      </div>
      <template v-else-if="events.showSkeleton.value">
        <div v-for="n in MAX_EVENTS" :key="n" class="flex h-8 items-center px-2" aria-hidden="true">
          <Skeleton class="h-3.5 w-2/3" />
        </div>
      </template>
      <p v-else-if="settings.data.value?.calendarConnected === false" class="text-13 text-muted-foreground">
        <button
          type="button"
          tabindex="-1"
          class="underline-offset-2 transition-colors hover:text-foreground hover:underline"
          @mousedown="keepFocus"
          @click="nav.go('settings')"
        >
          Calendar not connected
        </button>
      </p>
      <p v-else-if="events.data.value && nextEvents.length === 0" class="text-13 text-muted-foreground">
        Nothing else today
      </p>
      <div
        v-for="event in nextEvents"
        :key="event.id"
        class="flex h-8 items-center gap-2 rounded-md px-2"
        :class="{ 'bg-hover': isCurrent(event) }"
      >
        <span class="h-4 w-[3px] shrink-0 rounded-full" :style="{ backgroundColor: event.color }" />
        <span class="w-12 shrink-0 font-mono text-xs text-muted-foreground">
          {{ event.allDay ? 'All day' : formatClock(event.start) }}
        </span>
        <span class="min-w-0 truncate text-13 text-foreground">{{ event.title }}</span>
      </div>
    </div>

    <Button
      v-if="todayNote"
      variant="outline"
      class="mt-auto gap-2"
      tabindex="-1"
      @mousedown="keepFocus"
      @click="openTodayNote"
    >
      <NotebookPenIcon :size="16" :stroke-width="1.75" aria-hidden="true" />
      Today's note
      <KeyHint :keys="todayNote.keys ?? 'Ctrl+KeyT'" />
    </Button>
  </div>
</template>
