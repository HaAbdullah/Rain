<script setup lang="ts">
// A tiny picture of a screen with one zone filled: it says where "Left half" or "Top right" is
// without words. A zone that moves nothing (free) is a dashed outline.
import { computed } from 'vue'
import type { LayoutZone } from '@/data/types'
import { zoneInfo } from './layout'

const props = withDefaults(defineProps<{ zone: LayoutZone; width?: number }>(), { width: 20 })

const box = computed(() => zoneInfo(props.zone).box)
</script>

<template>
  <svg
    :width="width"
    :height="(width * 10) / 16"
    viewBox="0 0 16 10"
    class="shrink-0 text-muted-foreground"
    aria-hidden="true"
  >
    <rect
      x="0.5"
      y="0.5"
      width="15"
      height="9"
      rx="1.5"
      fill="none"
      stroke="currentColor"
      :stroke-dasharray="box ? undefined : '2 1.5'"
      opacity="0.7"
    />
    <rect
      v-if="box"
      :x="0.5 + box[0] * 15"
      :y="0.5 + box[1] * 9"
      :width="box[2] * 15"
      :height="box[3] * 9"
      fill="currentColor"
      class="text-foreground"
      opacity="0.8"
    />
  </svg>
</template>
