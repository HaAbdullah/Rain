<script setup lang="ts">
// One status glyph: a shape plus a colour (spec 5.4), so a state never depends on colour alone.
import type { Component } from 'vue'
import type { StatusKind } from '@/lib/status'
import {
  CircleCheckIcon,
  CircleDashedIcon,
  CircleDotIcon,
  CircleIcon,
  CirclePauseIcon,
  CircleXIcon,
  HandIcon,
  LoaderCircleIcon,
} from '@lucide/vue'
import { computed } from 'vue'
import { Tooltip, TooltipContent, TooltipProvider, TooltipTrigger } from '@/components/ui/tooltip'
import { statusColor } from '@/lib/status'

const props = withDefaults(
  defineProps<{
    status: StatusKind
    size?: 12 | 16
    /** Read out by screen readers and shown as the tooltip. */
    label?: string
  }>(),
  { size: 16 },
)

const glyphs: Record<StatusKind, Component> = {
  idle: CircleIcon,
  running: LoaderCircleIcon,
  waiting: HandIcon,
  success: CircleCheckIcon,
  failed: CircleXIcon,
  stale: CircleDashedIcon,
  paused: CirclePauseIcon,
  queued: CircleDotIcon,
}

// The spinner is the only loop in the app. It turns in 8 steps a second, not 60 smooth frames
// (a smooth turn cost about 9 % of a core), and tokens.css pauses it while Rain is hidden.
const glyphProps = computed(() => ({
  size: props.size,
  strokeWidth: 1.75,
  class: props.status === 'running' ? 'status-spin' : undefined,
}))
</script>

<template>
  <span
    class="inline-flex shrink-0"
    :role="label ? 'img' : undefined"
    :aria-label="label"
    :aria-hidden="label ? undefined : true"
    :style="{ color: statusColor[status] }"
  >
    <!-- The provider is local so a labelled dot works wherever it is placed; the nearest provider wins. -->
    <TooltipProvider v-if="label" :delay-duration="300">
      <Tooltip>
        <TooltipTrigger as-child>
          <component :is="glyphs[status]" v-bind="glyphProps" />
        </TooltipTrigger>
        <TooltipContent>{{ label }}</TooltipContent>
      </Tooltip>
    </TooltipProvider>
    <component :is="glyphs[status]" v-else v-bind="glyphProps" />
  </span>
</template>
