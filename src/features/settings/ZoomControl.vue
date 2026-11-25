<script setup lang="ts">
// The zoom control of the Appearance row: zoom out, the level, zoom in and reset. Enter on
// the row focuses the group; Left and Right step the zoom, Ctrl+0 is the global reset, and
// Enter puts the caret back in the search field. A click does not take focus.
import { RotateCcwIcon, ZoomInIcon, ZoomOutIcon } from '@lucide/vue'
import { useTemplateRef } from 'vue'
import { Button } from '@/components/ui/button'
import { useSearch } from '@/composables/useSearch'
import { resetZoom, zoom, zoomIn, zoomOut } from '@/composables/useZoom'
import { DEFAULT_ZOOM } from '@/composables/zoom-steps'

const search = useSearch()
const group = useTemplateRef<HTMLElement>('group')

/** Puts keyboard focus on the group, so Left and Right work. */
function focus() {
  group.value?.focus({ preventScroll: true })
}

function stop(event: KeyboardEvent) {
  event.preventDefault()
  event.stopPropagation()
}

function step(change: () => Promise<void>, event: KeyboardEvent) {
  stop(event)
  void change()
}

function done(event: KeyboardEvent) {
  stop(event)
  search.focus()
}

defineExpose({ focus })
</script>

<template>
  <div
    ref="group"
    role="group"
    aria-label="Zoom"
    tabindex="-1"
    class="flex items-center gap-1 rounded-lg outline-none focus-visible:ring-2 focus-visible:ring-ring"
    @keydown.left="step(zoomOut, $event)"
    @keydown.right="step(zoomIn, $event)"
    @keydown.enter="done"
  >
    <Button variant="ghost" size="icon" tabindex="-1" aria-label="Zoom out" @click="zoomOut()">
      <ZoomOutIcon :size="16" :stroke-width="1.75" aria-hidden="true" />
    </Button>
    <span class="w-12 text-center text-13 tabular-nums text-foreground" aria-live="polite">{{ zoom }}%</span>
    <Button variant="ghost" size="icon" tabindex="-1" aria-label="Zoom in" @click="zoomIn()">
      <ZoomInIcon :size="16" :stroke-width="1.75" aria-hidden="true" />
    </Button>
    <Button
      variant="ghost"
      size="icon"
      tabindex="-1"
      aria-label="Reset zoom"
      :disabled="zoom === DEFAULT_ZOOM"
      @click="resetZoom()"
    >
      <RotateCcwIcon :size="16" :stroke-width="1.75" aria-hidden="true" />
    </Button>
  </div>
</template>
