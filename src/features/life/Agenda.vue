<script setup lang="ts">
// Today's and tomorrow's calendar events, read-only. A missing calendar connection
// or a failed read is said here, inside the agenda, and never blocks the habits.
import { computed } from 'vue'
import EmptyState from '@/components/rain/EmptyState.vue'
import ErrorRow from '@/components/rain/ErrorRow.vue'
import SectionHeader from '@/components/rain/SectionHeader.vue'
import { Skeleton } from '@/components/ui/skeleton'
import type { CalendarEvent, ISODate } from '@/data/types'
import { formatClock, toISODate } from '@/lib/time'
import { addDays } from './dates'

const props = defineProps<{
  events: CalendarEvent[]
  /** Epoch ms; the event that is on now gets a fill. */
  now: number
  today: ISODate
  connected: boolean
  /** Nothing is known yet: paint no state at all. */
  loading: boolean
  /** The load has taken long enough to show the skeleton. */
  showSkeleton: boolean
  failed: boolean
}>()

const emit = defineEmits<{ connect: []; retry: [] }>()

const groups = computed(() => {
  const tomorrow = addDays(props.today, 1)
  const sorted = [...props.events].sort(
    // All-day events first, then by start time.
    (a, b) => Number(b.allDay) - Number(a.allDay) || a.start.localeCompare(b.start),
  )
  return [
    { id: 'today', title: 'Today', events: sorted.filter((event) => toISODate(new Date(event.start)) === props.today) },
    { id: 'tomorrow', title: 'Tomorrow', events: sorted.filter((event) => toISODate(new Date(event.start)) === tomorrow) },
  ].filter((group) => group.events.length > 0)
})

function isCurrent(event: CalendarEvent): boolean {
  return !event.allDay && Date.parse(event.start) <= props.now && props.now < Date.parse(event.end)
}
</script>

<template>
  <section aria-label="Agenda">
    <ErrorRow v-if="failed" message="Calendar unavailable" @retry="emit('retry')" />
    <EmptyState
      v-else-if="!connected"
      icon="calendar"
      title="Connect calendar"
      hint="Today's and tomorrow's events show here."
      action-label="Open settings"
      @action="emit('connect')"
    />
    <div v-else-if="loading" class="flex flex-col gap-2">
      <template v-if="showSkeleton">
        <Skeleton v-for="n in 3" :key="n" class="h-10" />
      </template>
    </div>
    <p v-else-if="groups.length === 0" class="px-2 py-3 text-13 text-muted-foreground">Nothing scheduled.</p>

    <template v-else>
      <div v-for="group in groups" :key="group.id">
        <SectionHeader :title="group.title" :count="group.events.length" />
        <div
          v-for="event in group.events"
          :key="event.id"
          class="flex h-10 items-center gap-3 rounded-lg px-2"
          :class="{ 'bg-hover': isCurrent(event) }"
        >
          <span class="h-5 w-[3px] shrink-0 rounded-full" :style="{ backgroundColor: event.color }" />
          <span class="w-14 shrink-0 font-mono text-xs text-muted-foreground">
            {{ event.allDay ? 'all day' : formatClock(event.start) }}
          </span>
          <span class="min-w-0 flex-1 truncate text-13 text-foreground">{{ event.title }}</span>
          <span v-if="event.location" class="max-w-24 shrink truncate text-xs text-muted-foreground">
            {{ event.location }}
          </span>
        </div>
      </div>
    </template>
  </section>
</template>
