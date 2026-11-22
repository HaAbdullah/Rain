<script setup lang="ts">
// The last 10 runs of a job as small squares, oldest on the left; missing runs are empty squares.
import { computed } from 'vue'
import type { JobRun } from '@/data/types'
import { runFill } from './runColor'

const SLOTS = 10

const props = defineProps<{
  /** Newest first, as the api gives them. */
  runs: JobRun[]
}>()

const slots = computed(() => {
  const shown = props.runs.slice(0, SLOTS).reverse()
  return [...Array.from({ length: SLOTS - shown.length }, () => null), ...shown]
})

const summary = computed(() => {
  const shown = props.runs.slice(0, SLOTS)
  const failed = shown.filter((run) => run.status === 'failed').length
  return `Last ${shown.length} runs, ${failed} failed`
})
</script>

<template>
  <div class="flex gap-0.5" role="img" :aria-label="summary" :title="summary">
    <span
      v-for="(run, index) in slots"
      :key="index"
      class="size-2 rounded-xs"
      :class="run ? runFill(run.status) : 'bg-surface-2'"
    />
  </div>
</template>
