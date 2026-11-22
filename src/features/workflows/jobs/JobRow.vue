<script setup lang="ts">
// One scheduled job in the list: status, name, schedule, next run and the run strip.
import { BotIcon, ClockIcon } from '@lucide/vue'
import { computed } from 'vue'
import RelTime from '@/components/rain/RelTime.vue'
import StatusDot from '@/components/rain/StatusDot.vue'
import type { ScheduledJob } from '@/data/types'
import { JOB_GRID } from './grid'
import RunStrip from './RunStrip.vue'

const props = defineProps<{
  job: ScheduledJob
  selected: boolean
}>()

/** Running, else the result of the last run, else idle. */
const status = computed(() => (props.job.running ? 'running' : (props.job.recentRuns[0]?.status ?? 'idle')))
</script>

<template>
  <div
    class="h-(--row-h) rounded-lg px-2 transition-colors"
    :class="[JOB_GRID, selected ? 'bg-selected' : 'hover:bg-hover', { 'opacity-60': !job.enabled }]"
  >
    <StatusDot :status="status" />

    <span class="flex min-w-0 items-center gap-2">
      <span class="truncate text-sm text-foreground">{{ job.name }}</span>
      <component
        :is="job.kind === 'claude' ? BotIcon : ClockIcon"
        :size="12"
        :stroke-width="1.75"
        class="shrink-0 text-text-3"
        aria-hidden="true"
      />
    </span>

    <span class="truncate text-13 text-muted-foreground">{{ job.scheduleLabel }}</span>

    <span class="text-13 text-muted-foreground">
      <RelTime v-if="job.enabled && job.nextRunAt" :iso="job.nextRunAt" />
    </span>

    <RunStrip :runs="job.recentRuns" />

    <span class="flex justify-center">
      <StatusDot v-if="!job.enabled" status="paused" label="Disabled" />
    </span>
  </div>
</template>
