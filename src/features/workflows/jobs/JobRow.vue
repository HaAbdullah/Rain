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

const fromApp = computed(() => props.job.source === 'claude-app')
/** What went wrong in the last run, for a job of the Claude app: shown on the row so a failure cannot hide. */
const problem = computed(() => {
  const last = props.job.recentRuns[0]
  return fromApp.value && last?.status === 'failed' ? (last.outcome ?? 'Last run failed') : undefined
})
</script>

<template>
  <div
    class="h-(--row-h) rounded-lg px-2 transition-colors"
    :class="[JOB_GRID, selected ? 'bg-selected' : 'hover:bg-hover', { 'opacity-60': !job.enabled }]"
  >
    <StatusDot :status="status" />

    <span class="flex min-w-0 items-center gap-2">
      <span class="truncate text-sm text-foreground" :class="{ 'shrink-0 max-w-[60%]': problem }">{{ job.name }}</span>
      <component
        :is="job.kind === 'claude' ? BotIcon : ClockIcon"
        :size="12"
        :stroke-width="1.75"
        class="shrink-0 text-text-3"
        aria-hidden="true"
      />
      <span
        v-if="fromApp"
        class="shrink-0 rounded-sm bg-surface-2 px-1.5 text-xs text-muted-foreground"
        title="Runs in the Claude app"
      >
        Claude app
      </span>
      <span v-if="problem" class="min-w-0 truncate text-xs text-status-failed" :title="problem">{{ problem }}</span>
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
