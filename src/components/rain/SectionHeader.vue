<script setup lang="ts">
// The 28 px header above a group of rows or tiles.
defineProps<{
  title: string
  count?: number
}>()
</script>

<template>
  <div class="flex h-7 shrink-0 items-center gap-2 px-2 text-xs font-medium text-muted-foreground">
    <span class="truncate">{{ title }}</span>
    <span v-if="count !== undefined" class="text-text-3">{{ count }}</span>
  </div>
</template>
