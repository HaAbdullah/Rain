<script setup lang="ts">
// The steps of a multi-step launch (a pool, a dev environment), one line each, with live status.
import type { LaunchStep } from '@/data/types'
import RIcon from './RIcon.vue'
import StatusDot from './StatusDot.vue'

defineProps<{ steps: LaunchStep[] }>()
</script>

<template>
  <ul class="flex flex-col">
    <li v-for="(step, i) in steps" :key="i" class="flex h-8 items-center gap-3">
      <RIcon :icon="step.icon" :size="16" />
      <span class="min-w-0 truncate text-sm text-foreground" :class="{ 'flex-1': !step.error }">{{ step.label }}</span>
      <span
        v-if="step.error"
        class="min-w-0 flex-1 truncate text-right text-13 text-status-failed"
        :title="step.error"
      >
        {{ step.error }}
      </span>
      <StatusDot :status="step.status" />
    </li>
  </ul>
</template>
