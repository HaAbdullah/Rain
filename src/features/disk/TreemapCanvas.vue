<script setup lang="ts">
// The treemap of the open folder on one canvas, with the colour legend under it.
// The canvas is sized for the screen's pixel ratio and redrawn from one debounced ResizeObserver.
import { useDebounceFn, useResizeObserver } from '@vueuse/core'
import { onMounted, useTemplateRef, watch } from 'vue'
import type { DiskNode } from '@/data/types'
import { keepFocus } from '@/lib/focus'
import {
  CATEGORIES,
  drawTreemap,
  hitTest,
  layoutTreemap,
  readPalette,
  type Palette,
  type TreemapRect,
} from './treemap'

const props = defineProps<{
  folder: DiskNode
  /** Path of the selected child of the folder: it gets the outline. */
  selectedPath: string | null
}>()

const emit = defineEmits<{
  /** A click on a rectangle: the path of its top-level ancestor, which is a row of the list. */
  select: [path: string]
  open: [path: string]
}>()

const RESIZE_DEBOUNCE_MS = 100

const host = useTemplateRef<HTMLElement>('host')
const canvas = useTemplateRef<HTMLCanvasElement>('canvas')

let palette: Palette | null = null
let layout: TreemapRect | null = null
let width = 0
let height = 0

function paint() {
  const ctx = canvas.value?.getContext('2d')
  if (!ctx || !palette || !layout) return
  ctx.setTransform(window.devicePixelRatio, 0, 0, window.devicePixelRatio, 0, 0)
  drawTreemap(ctx, layout, palette, props.selectedPath)
}

/** Sizes the canvas to its box, lays the folder out again and draws. */
function relayout() {
  const box = host.value
  const el = canvas.value
  if (!box || !el) return
  width = box.clientWidth
  height = box.clientHeight
  if (width === 0 || height === 0) return
  el.width = Math.round(width * window.devicePixelRatio)
  el.height = Math.round(height * window.devicePixelRatio)
  layout = layoutTreemap(props.folder, width, height)
  paint()
}

onMounted(() => {
  palette = readPalette()
  relayout()
})

useResizeObserver(host, useDebounceFn(relayout, RESIZE_DEBOUNCE_MS))
watch(() => props.folder, relayout)
watch(() => props.selectedPath, paint)

function topLevelAt(event: MouseEvent): TreemapRect | null {
  return layout ? hitTest(layout, event.offsetX, event.offsetY) : null
}

function onClick(event: MouseEvent) {
  const hit = topLevelAt(event)
  if (hit) emit('select', hit.data.path)
}

function onDoubleClick(event: MouseEvent) {
  const hit = topLevelAt(event)
  if (hit) emit('open', hit.data.path)
}
</script>

<template>
  <div class="flex h-full min-h-0 min-w-0 flex-col">
    <div ref="host" class="relative min-h-0 flex-1 bg-surface-0">
      <canvas
        ref="canvas"
        class="absolute inset-0 size-full"
        @mousedown="keepFocus"
        @click="onClick"
        @dblclick="onDoubleClick"
      />
    </div>

    <ul class="flex h-6 shrink-0 items-center gap-4 overflow-hidden border-t border-border px-3" aria-label="File types">
      <li v-for="category in CATEGORIES" :key="category.id" class="flex shrink-0 items-center gap-1.5 text-xs text-muted-foreground">
        <span class="size-2.5 rounded-xs opacity-75" :style="{ backgroundColor: `var(--file-${category.id})` }" />
        {{ category.label }}
      </li>
    </ul>
  </div>
</template>
