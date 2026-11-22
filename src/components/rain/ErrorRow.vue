<script setup lang="ts">
// Inline error with a retry button. Rain has no toasts: a failed source says so where its data would be.
import { TriangleAlertIcon } from '@lucide/vue'
import { Button } from '@/components/ui/button'
import { keepFocus } from '@/lib/focus'

withDefaults(
  defineProps<{
    message: string
    retryLabel?: string
  }>(),
  { retryLabel: 'Retry' },
)

const emit = defineEmits<{ retry: [] }>()
</script>

<template>
  <div class="flex h-10 shrink-0 items-center gap-3 rounded-lg px-2" role="alert">
    <TriangleAlertIcon :size="16" :stroke-width="1.75" class="shrink-0 text-status-failed" aria-hidden="true" />
    <span class="min-w-0 flex-1 truncate text-sm text-foreground">{{ message }}</span>
    <Button variant="ghost" size="sm" tabindex="-1" @mousedown="keepFocus" @click="emit('retry')">
      {{ retryLabel }}
    </Button>
  </div>
</template>
