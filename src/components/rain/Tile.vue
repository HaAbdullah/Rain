<script setup lang="ts">
// The frame of a grid card (project card, game tile). Selection is a ring, not a fill,
// so a cover image stays fully visible.
withDefaults(
  defineProps<{
    selected: boolean
    dimmed?: boolean
    /** 'success' marks a running item with a green border. */
    ring?: 'none' | 'success'
  }>(),
  { ring: 'none' },
)
</script>

<template>
  <div
    class="relative h-full min-w-0 rounded-lg border bg-surface-2 outline-offset-0 transition-colors hover:bg-hover"
    :class="[
      ring === 'success' ? 'border-status-success' : 'border-border',
      selected ? 'outline-2 outline-ring' : 'outline-none',
      { 'opacity-50': dimmed },
    ]"
  >
    <slot />
  </div>
</template>
