<script setup lang="ts">
// A quiet one-line note about a source that is off or failed, with the backend's own text.
// Not an error row: nothing here can be retried, and the screen around it still works.
import { InfoIcon, TriangleAlertIcon } from '@lucide/vue'
import type { SourceStatus } from '@/data/types'

defineProps<{ source: SourceStatus }>()
</script>

<template>
  <p class="flex h-8 shrink-0 items-center gap-2 px-2 text-13 text-muted-foreground" :title="source.message">
    <TriangleAlertIcon
      v-if="source.state === 'failed'"
      :size="14"
      :stroke-width="1.75"
      class="shrink-0 text-status-waiting"
      aria-hidden="true"
    />
    <InfoIcon v-else :size="14" :stroke-width="1.75" class="shrink-0" aria-hidden="true" />
    <span class="min-w-0 truncate">{{ source.message }}</span>
  </p>
</template>
