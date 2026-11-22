<script setup lang="ts">
// A thin share bar (for example a folder's part of its parent). A plain element:
// it shows a fixed share, not progress, so the Progress component is not used.
import { computed } from 'vue'

const props = withDefaults(
  defineProps<{
    /** Share from 0 to 1. */
    value: number
    tone?: 'neutral' | 'primary'
    /** Width in px. Without it the bar fills its container. */
    width?: number
  }>(),
  { tone: 'neutral' },
)

const percent = computed(() => Math.min(1, Math.max(0, props.value)) * 100)
</script>

<template>
  <div
    class="h-1 shrink-0 overflow-hidden rounded-full bg-border"
    :class="{ 'w-full': width === undefined }"
    :style="width === undefined ? undefined : { width: `${width}px` }"
    role="meter"
    aria-valuemin="0"
    aria-valuemax="100"
    :aria-valuenow="Math.round(percent)"
  >
    <div
      class="h-full rounded-full"
      :class="tone === 'primary' ? 'bg-ring' : 'bg-muted-foreground'"
      :style="{ width: `${percent}%` }"
    />
  </div>
</template>
