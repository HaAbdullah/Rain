<script setup lang="ts">
// The result of an action ("Launched Web dev", "Executable not found"), shown as
// a small pill at the bottom of the window. It exists only while there is
// something to say, so the frame has no permanent bottom bar.
import { CircleAlertIcon, CircleCheckIcon, InfoIcon } from '@lucide/vue'
import { computed } from 'vue'
import { useNotice } from '@/composables/useNotice'

const { notice } = useNotice()

const icon = computed(() => {
  if (notice.value?.kind === 'success') return CircleCheckIcon
  if (notice.value?.kind === 'error') return CircleAlertIcon
  return InfoIcon
})
const iconColor = computed(() => {
  if (notice.value?.kind === 'success') return 'text-status-success'
  if (notice.value?.kind === 'error') return 'text-status-failed'
  return 'text-muted-foreground'
})
</script>

<template>
  <!-- The wrapper spans the window and ignores the pointer, so the pill never blocks a click. -->
  <div class="pointer-events-none absolute inset-x-0 bottom-4 flex justify-center px-4">
    <Transition
      enter-active-class="transition-opacity"
      leave-active-class="transition-opacity"
      enter-from-class="opacity-0"
      leave-to-class="opacity-0"
    >
      <div
        v-if="notice"
        role="status"
        class="flex h-9 max-w-full items-center gap-2 rounded-lg border border-border-strong bg-popover px-3 text-13 text-foreground shadow-(--shadow-overlay)"
      >
        <component :is="icon" class="size-4 shrink-0" :class="iconColor" />
        <span class="truncate">{{ notice.text }}</span>
      </div>
    </Transition>
  </div>
</template>
