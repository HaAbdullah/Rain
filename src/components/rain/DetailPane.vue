<script setup lang="ts">
// The body of the fixed detail pane: a title line, content that scrolls,
// and an `actions` slot that stays pinned to the bottom.
import type { IconRef } from '@/data/types'
import type { StatusKind } from '@/lib/status'
import RIcon from './RIcon.vue'
import StatusDot from './StatusDot.vue'

defineProps<{
  title: string
  icon?: IconRef
  status?: StatusKind
}>()
</script>

<template>
  <section class="flex h-full min-h-0 flex-col bg-surface-1">
    <header class="flex shrink-0 items-center gap-3 px-4 pt-4 pb-3">
      <RIcon v-if="icon" :icon="icon" :size="20" />
      <h2 class="selectable min-w-0 flex-1 truncate text-base font-semibold text-foreground">{{ title }}</h2>
      <StatusDot v-if="status" :status="status" />
    </header>

    <div class="min-h-0 flex-1 overflow-y-auto px-4 pb-4">
      <slot />
    </div>

    <footer v-if="$slots.actions" class="flex shrink-0 flex-wrap items-center gap-2 border-t border-border p-4">
      <slot name="actions" />
    </footer>
  </section>
</template>
