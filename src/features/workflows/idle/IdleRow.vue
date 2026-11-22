<script setup lang="ts">
// One idle job in the list: state, name, what it is doing, and its resource and progress.
import { CpuIcon, GpuIcon } from '@lucide/vue'
import Meter from '@/components/rain/Meter.vue'
import StatusDot from '@/components/rain/StatusDot.vue'
import type { IdleJob } from '@/data/types'
import { idleStateText } from './idleState'

defineProps<{
  job: IdleJob
  selected: boolean
}>()
</script>

<template>
  <div
    class="flex h-(--row-lg-h) items-center gap-3 rounded-lg px-2 transition-colors"
    :class="[selected ? 'bg-selected' : 'hover:bg-hover', { 'opacity-60': !job.enabled }]"
  >
    <StatusDot :status="job.state" />

    <div class="flex min-w-0 flex-1 flex-col justify-center">
      <span class="truncate text-sm text-foreground">{{ job.name }}</span>
      <span class="truncate text-13 text-muted-foreground">{{ idleStateText(job) }}</span>
    </div>

    <component
      :is="job.resource === 'gpu' ? GpuIcon : CpuIcon"
      :size="16"
      :stroke-width="1.75"
      class="shrink-0 text-text-3"
      :aria-label="job.resource === 'gpu' ? 'Uses the GPU' : 'Uses the CPU'"
    />
    <Meter :value="job.progress" tone="primary" :width="160" />
  </div>
</template>
