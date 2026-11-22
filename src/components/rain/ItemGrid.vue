<script setup lang="ts" generic="T extends { id: string }">
// A grid of grouped tiles (projects, games). Like ItemList, the screen owns the selection
// and draws each tile through the `tile` slot; this component lays the tiles out, keeps the
// selected one in view, reports the live column count for 2D arrow movement, and never
// takes DOM focus. It does not scroll by itself: the ScreenFrame main area scrolls it.
import type { SelGroup } from '@/composables/useSelection'
import { useResizeObserver } from '@vueuse/core'
import { computed, onMounted, ref, useTemplateRef } from 'vue'
import { keepFocus } from '@/lib/focus'
import { useItemPointer } from '@/lib/item-pointer'
import { useRevealSelected } from '@/lib/reveal'
import SectionHeader from './SectionHeader.vue'

const props = withDefaults(
  defineProps<{
    groups: SelGroup<T>[]
    selectedId: string | null
    /** Smallest tile width in px; the grid fits as many columns as it can. Ignored when `columns` is set. */
    minCol?: number
    /** A fixed column count. A group's own `columns` wins over this. */
    columns?: number
    /** Space between tiles in px. */
    gap?: number
  }>(),
  { minCol: 160, gap: 12 },
)

const emit = defineEmits<{
  'update:selectedId': [id: string]
  activate: [item: T]
  context: [item: T, event: MouseEvent]
}>()

defineSlots<{
  tile(props: { item: T; selected: boolean }): unknown
}>()

const visibleGroups = computed(() => props.groups.filter((group) => group.items.length > 0))

// ---- live column count ----
const content = useTemplateRef<HTMLElement>('content')
const width = ref(0)

onMounted(() => {
  width.value = content.value?.clientWidth ?? 0
})
useResizeObserver(content, ([entry]) => {
  width.value = entry.contentRect.width
})

/** Columns of a group without its own count. The same arithmetic as CSS `repeat(auto-fill, minmax(minCol, 1fr))`. */
const liveColumns = computed(() => {
  if (props.columns !== undefined) return props.columns
  return Math.max(1, Math.floor((width.value + props.gap) / (props.minCol + props.gap)))
})

function columnsOf(group: SelGroup<T>): number {
  return group.columns ?? liveColumns.value
}

function gridStyle(group: SelGroup<T>) {
  const fixed = group.columns ?? props.columns
  return {
    gap: `${props.gap}px`,
    gridTemplateColumns:
      fixed !== undefined
        ? `repeat(${fixed}, minmax(0, 1fr))`
        : `repeat(auto-fill, minmax(${props.minCol}px, 1fr))`,
  }
}

defineExpose({
  /** Live column count, for `useSelection({ columns })`. */
  columns: liveColumns,
})

// ---- mouse and scrolling ----
const pointer = useItemPointer<T>({
  selectedId: () => props.selectedId,
  clickRuns: () => false,
  select: (id) => emit('update:selectedId', id),
  activate: (item) => emit('activate', item),
  context: (item, event) => emit('context', item, event),
})

useRevealSelected(() => content.value, () => props.groups, () => props.selectedId)
</script>

<template>
  <!-- The 16 px padding also leaves room for the 2 px selection ring of the outer tiles. -->
  <div class="shrink-0 p-4" @mousedown="keepFocus">
    <div ref="content" class="flex flex-col gap-4" role="listbox">
      <div
        v-for="group in visibleGroups"
        :key="group.id"
        data-group
        role="group"
        :aria-label="group.title"
      >
        <!-- Pulled left by its own 8 px padding so the title lines up with the tiles. -->
        <SectionHeader
          v-if="group.title"
          data-group-header
          class="-ml-2 scroll-mt-4"
          :title="group.title"
          :count="group.count"
        />
        <div class="grid" :style="gridStyle(group)">
          <div
            v-for="(item, i) in group.items"
            :key="item.id"
            :data-item-id="item.id"
            :data-group-start="i < columnsOf(group) ? '' : undefined"
            role="option"
            :aria-selected="item.id === selectedId"
            class="min-w-0 scroll-my-4"
            @click="pointer.onClick(item, $event)"
            @dblclick="pointer.onDoubleClick(item)"
            @contextmenu="pointer.onContextMenu(item, $event)"
          >
            <slot name="tile" :item="item" :selected="item.id === selectedId" />
          </div>
        </div>
      </div>
    </div>
  </div>
</template>
