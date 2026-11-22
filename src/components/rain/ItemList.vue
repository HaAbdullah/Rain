<script setup lang="ts" generic="T extends { id: string }">
// A scrollable list of grouped rows. The screen owns the selection (useSelection) and
// draws each row through the `row` slot; this component adds the group headers, keeps
// the selected row in view and turns mouse input into select / activate / context events.
// It never takes DOM focus: that stays in the search field.
import type { SelGroup } from '@/composables/useSelection'
import type { RowSize } from '@/lib/list-context'
import { computed, provide, toRef, useTemplateRef } from 'vue'
import { ScrollArea } from '@/components/ui/scroll-area'
import { keepFocus } from '@/lib/focus'
import { useItemPointer } from '@/lib/item-pointer'
import { rowSizeKey } from '@/lib/list-context'
import { useRevealSelected } from '@/lib/reveal'
import SectionHeader from './SectionHeader.vue'

const props = withDefaults(
  defineProps<{
    groups: SelGroup<T>[]
    selectedId: string | null
    /** Default size of the ListRows inside: 40 px ('md') or 56 px ('lg'). */
    rowSize?: RowSize
    /** One click runs the primary action (Home). Otherwise click selects and double-click runs. */
    clickRuns?: boolean
  }>(),
  { rowSize: 'md', clickRuns: false },
)

const emit = defineEmits<{
  'update:selectedId': [id: string]
  activate: [item: T]
  context: [item: T, event: MouseEvent]
}>()

defineSlots<{
  /** `index` counts across all groups, starting at 0. */
  row(props: { item: T; selected: boolean; index: number }): unknown
}>()

provide(rowSizeKey, toRef(props, 'rowSize'))

/** The groups that have rows, each with the list-wide index of its first row. */
const visibleGroups = computed(() => {
  let offset = 0
  return props.groups.flatMap((group) => {
    const entry = { group, offset }
    offset += group.items.length
    return group.items.length > 0 ? [entry] : []
  })
})

const pointer = useItemPointer<T>({
  selectedId: () => props.selectedId,
  clickRuns: () => props.clickRuns,
  select: (id) => emit('update:selectedId', id),
  activate: (item) => emit('activate', item),
  context: (item, event) => emit('context', item, event),
})

const content = useTemplateRef<HTMLElement>('content')

useRevealSelected(() => content.value, () => props.groups, () => props.selectedId)
</script>

<template>
  <!--
    Full height of the parent. In a flex column with an ErrorRow or a status line above it,
    the high shrink factor makes the list give up the room, so the line above keeps its height.
  -->
  <ScrollArea class="h-full min-h-0 shrink-[999]" @mousedown="keepFocus">
    <div ref="content" class="p-2" role="listbox">
      <div
        v-for="{ group, offset } in visibleGroups"
        :key="group.id"
        data-group
        role="group"
        :aria-label="group.title"
      >
        <SectionHeader
          v-if="group.title"
          data-group-header
          class="scroll-mt-2"
          :title="group.title"
          :count="group.count"
        />
        <div
          v-for="(item, i) in group.items"
          :key="item.id"
          :data-item-id="item.id"
          :data-group-start="i === 0 ? '' : undefined"
          role="option"
          :aria-selected="item.id === selectedId"
          class="scroll-my-2"
          @click="pointer.onClick(item, $event)"
          @dblclick="pointer.onDoubleClick(item)"
          @contextmenu="pointer.onContextMenu(item, $event)"
        >
          <slot name="row" :item="item" :selected="item.id === selectedId" :index="offset + i" />
        </div>
      </div>
    </div>
  </ScrollArea>
</template>
