<script setup lang="ts">
// Section navigation (spec 3.3): one icon button per route, tooltips with name and key.
// The icons at the top can be dragged up or down; the order is remembered (useRailOrder) and
// the keys Ctrl+1, Ctrl+2 ... follow it. The icons at the bottom stay where they are.
import { CloudRainIcon } from '@lucide/vue'
import { computed, ref, useTemplateRef } from 'vue'
import { Tooltip, TooltipContent, TooltipTrigger } from '@/components/ui/tooltip'
import { useNeedsYou } from '@/composables/useNeedsYou'
import { keepFocus } from '@/lib/focus'
import { lucideIcon } from '@/lib/icon-map'
import { dropSlot, slotChangesOrder } from '@/nav/rail-order'
import { routes, type RouteName } from '@/nav/routes'
import { useNav } from '@/nav/useNav'
import { moveSectionToSlot, railOrder, railTop, routeCombo } from '@/nav/useRailOrder'
import KeyHint from './KeyHint.vue'

const nav = useNav()
const { waiting, hasFailedJob } = useNeedsYou()

const rails = computed(() => [
  { name: 'top', defs: railTop.value, class: 'relative', movable: true },
  {
    name: 'bottom',
    defs: routes.filter((def) => def.rail === 'bottom'),
    class: 'mt-auto pb-2',
    movable: false,
  },
])

// ---- drag to reorder ----

/** The pointer must travel this far before a press becomes a drag, so a click stays a click. */
const DRAG_START_PX = 4

const topRail = useTemplateRef<HTMLElement[]>('rail')
let press: { name: RouteName; startY: number } | null = null
const dragging = ref<RouteName | null>(null)
/** Where the dragged icon would land (a gap between icons), and where to draw the line for it. */
const slot = ref(0)
const lineTop = ref<number | null>(null)
// The click that ends a drag must not open the section.
let swallowClick = false

function topButtons(): HTMLElement[] {
  const container = topRail.value?.find((element) => element.dataset.rail === 'top')
  return container ? [...container.querySelectorAll<HTMLElement>('[data-rail-id]')] : []
}

function onPointerDown(event: PointerEvent, name: RouteName, movable: boolean) {
  swallowClick = false
  if (!movable || event.button !== 0) return
  press = { name, startY: event.clientY }
  ;(event.currentTarget as HTMLElement).setPointerCapture(event.pointerId)
}

function onPointerMove(event: PointerEvent) {
  if (!press) return
  if (!dragging.value) {
    if (Math.abs(event.clientY - press.startY) < DRAG_START_PX) return
    dragging.value = press.name
  }
  const buttons = topButtons()
  const rects = buttons.map((button) => button.getBoundingClientRect())
  slot.value = dropSlot(
    rects.map((rect) => rect.top + rect.height / 2),
    event.clientY,
  )
  if (!slotChangesOrder(railOrder.value, dragging.value, slot.value) || rects.length === 0) {
    lineTop.value = null
    return
  }
  const containerTop = buttons[0].parentElement?.getBoundingClientRect().top ?? 0
  // Half a gap (4 px) outside the icon the line sits next to.
  const edge = slot.value < rects.length ? rects[slot.value].top - 4 : rects[rects.length - 1].bottom + 4
  lineTop.value = edge - containerTop
}

function endDrag(commit: boolean) {
  if (dragging.value) {
    swallowClick = true
    if (commit) moveSectionToSlot(dragging.value, slot.value)
  }
  press = null
  dragging.value = null
  lineTop.value = null
}

function onClick(name: RouteName) {
  if (swallowClick) {
    swallowClick = false
    return
  }
  nav.go(name)
}
</script>

<template>
  <nav class="flex h-full flex-col border-r border-sidebar-border bg-sidebar">
    <div data-tauri-drag-region class="flex h-14 shrink-0 items-center justify-center">
      <CloudRainIcon class="pointer-events-none size-5 text-muted-foreground" :stroke-width="1.75" />
    </div>

    <div
      v-for="rail in rails"
      :key="rail.name"
      ref="rail"
      :data-rail="rail.name"
      class="flex flex-col items-center gap-2"
      :class="rail.class"
    >
      <Tooltip v-for="def in rail.defs" :key="def.name">
        <TooltipTrigger as-child>
          <button
            type="button"
            tabindex="-1"
            :data-rail-id="rail.movable ? def.name : undefined"
            :aria-label="def.title"
            :aria-current="nav.displayed.value === def.name ? 'page' : undefined"
            class="relative flex size-10 items-center justify-center rounded-lg transition-colors"
            :class="[
              nav.displayed.value === def.name
                ? 'bg-selected text-foreground'
                : 'text-sidebar-foreground hover:bg-hover',
              rail.movable && 'touch-none',
              dragging === def.name && 'opacity-50',
            ]"
            @mousedown="keepFocus"
            @pointerdown="onPointerDown($event, def.name, rail.movable)"
            @pointermove="onPointerMove"
            @pointerup="endDrag(true)"
            @pointercancel="endDrag(false)"
            @click="onClick(def.name)"
          >
            <component :is="lucideIcon(def.icon)" class="size-5" :stroke-width="1.75" />
            <template v-if="def.name === 'workflows'">
              <span
                v-if="waiting > 0"
                class="absolute -top-0.5 -right-0.5 flex h-4 min-w-4 items-center justify-center rounded-full bg-status-waiting px-1 text-xs font-semibold text-background"
              >
                {{ waiting }}
              </span>
              <span
                v-else-if="hasFailedJob"
                class="absolute top-1 right-1 size-2 rounded-full bg-status-failed"
              />
            </template>
          </button>
        </TooltipTrigger>
        <TooltipContent v-if="dragging === null" side="right">
          <span>{{ def.title }}</span>
          <KeyHint v-if="routeCombo(def)" :keys="routeCombo(def)!" />
        </TooltipContent>
      </Tooltip>

      <!-- Where the dragged icon will land. -->
      <div
        v-if="rail.movable && lineTop !== null"
        class="pointer-events-none absolute inset-x-2 h-0.5 rounded-full bg-ring"
        :style="{ top: `${lineTop - 1}px` }"
        aria-hidden="true"
      />
    </div>
  </nav>
</template>
