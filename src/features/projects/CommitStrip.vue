<script setup lang="ts">
// The last 14 days of commits as thin bars, oldest first; today's bar is the brightest.
import { computed } from 'vue'

const props = defineProps<{ counts: number[] }>()

const total = computed(() => props.counts.reduce((sum, count) => sum + count, 0))
const peak = computed(() => Math.max(1, ...props.counts))
const label = computed(() => `${total.value} commits in 14 days`)

function barClass(count: number, index: number): string {
  if (index === props.counts.length - 1) return 'bg-heat-4'
  return count === 0 ? 'bg-heat-0' : 'bg-heat-2'
}
</script>

<template>
  <div class="flex h-6 items-end gap-0.5" role="img" :aria-label="label" :title="label">
    <span
      v-for="(count, index) in counts"
      :key="index"
      class="min-h-0.5 min-w-0 flex-1 rounded-[1px]"
      :class="barClass(count, index)"
      :style="{ height: `${(count / peak) * 100}%` }"
    />
  </div>
</template>
