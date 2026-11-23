<script setup lang="ts">
// Draws any IconRef: a Lucide icon, a brand mark, an image, or a letter tile
// for everything that has no icon of its own.
import type { IconRef } from '@/data/types'
import { computed } from 'vue'
import { useImageFallback } from '@/composables/useImageFallback'
import { brandIcon } from '@/lib/brand-icons'
import { lucideIcon } from '@/lib/icon-map'

const props = withDefaults(
  defineProps<{
    icon: IconRef
    /** Edge length in px. */
    size?: number
    /** The name behind the icon. A picture that fails to load becomes a letter tile of its first letter. */
    label?: string
  }>(),
  { size: 20 },
)

const CHART_TOKENS = 5

/** Picks one of the --chart-N tokens from the name, so a name always gets the same colour. */
function tintFor(name: string): string {
  let hash = 0
  for (let i = 0; i < name.length; i += 1) hash = (hash * 31 + name.charCodeAt(i)) >>> 0
  return `var(--chart-${(hash % CHART_TOKENS) + 1})`
}

const brand = computed(() => (props.icon.kind === 'brand' ? brandIcon(props.icon.value) : undefined))

const picture = useImageFallback(() => (props.icon.kind === 'url' ? props.icon.value : undefined))

// A brand slug that Rain does not ship, and a picture that did not load, fall back to a letter tile.
const kind = computed(() => {
  if (props.icon.kind === 'brand' && !brand.value) return 'letter'
  return props.icon.kind === 'url' && picture.failed.value ? 'letter' : props.icon.kind
})

/** The text the tile is made from. An address says nothing about the thing, so a picture uses its label. */
const tileName = computed(() => (props.icon.kind === 'url' ? (props.icon.fallback ?? props.label ?? '') : props.icon.value))

const box = computed(() => ({ width: `${props.size}px`, height: `${props.size}px` }))

const letterStyle = computed(() => ({
  ...box.value,
  // 12 px is the smallest text size in the app.
  fontSize: `${Math.max(12, Math.round(props.size * 0.5))}px`,
  backgroundColor: `color-mix(in srgb, ${props.icon.tint ?? tintFor(tileName.value || props.icon.value)} 40%, var(--surface-0))`,
}))
</script>

<template>
  <component
    :is="lucideIcon(icon.value)"
    v-if="kind === 'lucide'"
    :size="size"
    :stroke-width="1.75"
    class="shrink-0"
    :style="{ color: icon.tint }"
    aria-hidden="true"
  />

  <svg
    v-else-if="kind === 'brand' && brand"
    viewBox="0 0 24 24"
    fill="currentColor"
    class="shrink-0"
    :style="{ ...box, color: icon.tint }"
    aria-hidden="true"
  >
    <path :d="brand.path" />
  </svg>

  <img
    v-else-if="kind === 'url'"
    :src="icon.value"
    :width="size"
    :height="size"
    alt=""
    loading="lazy"
    decoding="async"
    draggable="false"
    class="shrink-0 rounded-sm object-cover"
    :style="box"
    @error="picture.onError"
  >

  <span
    v-else
    class="inline-flex shrink-0 items-center justify-center font-semibold leading-none text-foreground uppercase"
    :class="size >= 24 ? 'rounded-md' : 'rounded-sm'"
    :style="letterStyle"
    aria-hidden="true"
  >
    {{ tileName.trim().charAt(0) || '?' }}
  </span>
</template>
