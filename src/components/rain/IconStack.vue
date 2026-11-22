<script setup lang="ts">
// A row of small icons (a pool's members, a project's tools) with "+n" for the ones that do not fit.
import type { IconRef } from '@/data/types'
import { computed } from 'vue'
import RIcon from './RIcon.vue'

const props = withDefaults(
  defineProps<{
    icons: IconRef[]
    max?: number
    size?: number
  }>(),
  { max: 4, size: 16 },
)

const shown = computed(() => props.icons.slice(0, props.max))
const hidden = computed(() => props.icons.length - shown.value.length)
</script>

<template>
  <span class="inline-flex shrink-0 items-center gap-1">
    <RIcon v-for="(icon, i) in shown" :key="i" :icon="icon" :size="size" />
    <span v-if="hidden > 0" class="text-xs text-muted-foreground">+{{ hidden }}</span>
  </span>
</template>
