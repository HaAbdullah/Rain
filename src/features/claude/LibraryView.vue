<script setup lang="ts">
// One list sub-view of the Claude screen (Skills, Memory, Instructions, Agents): the files grouped
// by scope under a strip, and the selected file in the pane.
import { computed } from 'vue'
import { runPrimary } from '@/actions/context'
import type { ActionTarget } from '@/actions/types'
import EmptyState from '@/components/rain/EmptyState.vue'
import ErrorRow from '@/components/rain/ErrorRow.vue'
import ItemList from '@/components/rain/ItemList.vue'
import ScreenFrame from '@/components/rain/ScreenFrame.vue'
import { Skeleton } from '@/components/ui/skeleton'
import { useActionPanel } from '@/composables/useActionPanel'
import { useSectionQuery } from '@/composables/useSearch'
import { useSelection } from '@/composables/useSelection'
import type { LibraryItem } from '@/data/types'
import ItemDetail from './ItemDetail.vue'
import ItemRow from './ItemRow.vue'
import { EMPTY_TEXT, groupsOf, itemsOfView, type ListView } from './model'
import { useClaudeLibrary } from './resources'

const props = defineProps<{ view: ListView }>()
defineSlots<{ strip(): unknown }>()

const library = useClaudeLibrary()
const query = useSectionQuery('claude')
const panel = useActionPanel()

const hasData = computed(() => library.data.value !== undefined)
const groups = computed(() => groupsOf(library.data.value?.items ?? [], props.view, query.value))
const shownCount = computed(() => groups.value.reduce((sum, group) => sum + group.items.length, 0))
const totalCount = computed(() => itemsOfView(library.data.value?.items ?? [], props.view).length)
const showSkeleton = computed(() => !hasData.value && library.showSkeleton.value)

const { selectedId, selected, move, page, group, select } = useSelection(groups, {
  resetOn: query,
  persistKey: `claude.${props.view}`,
})

const target = computed<ActionTarget | null>(() =>
  selected.value ? { type: 'claudeItem', id: selected.value.id, item: selected.value } : null,
)

function openPanel(item: LibraryItem, event: MouseEvent) {
  select(item.id)
  if (target.value) panel.open(target.value, event)
}

defineExpose({ target, move, page, group })
</script>

<template>
  <ScreenFrame pane="wide">
    <template #strip><slot name="strip" /></template>

    <ErrorRow
      v-if="library.error.value"
      class="m-2"
      message="Claude library unavailable"
      :detail="library.error.value.message"
      @retry="library.reload"
    />
    <p v-if="library.data.value?.truncated" class="px-4 pt-2 text-xs text-text-3">
      There are very many files. The list shows the first ones only
    </p>
    <div v-if="showSkeleton" class="flex flex-col gap-1 p-2" aria-hidden="true">
      <Skeleton v-for="row in 6" :key="row" class="h-(--row-lg-h) rounded-lg" />
    </div>
    <EmptyState
      v-else-if="hasData && library.data.value?.found === false"
      icon="library"
      title="No Claude folder found"
      hint="Claude Code has not run on this PC yet, so there is nothing to show"
    />
    <EmptyState
      v-else-if="hasData && totalCount === 0"
      icon="library"
      :title="EMPTY_TEXT[view].title"
      :hint="EMPTY_TEXT[view].hint"
    />
    <EmptyState v-else-if="hasData && shownCount === 0" icon="search-x" title="No match" />
    <ItemList
      v-else-if="hasData"
      :groups="groups"
      :selected-id="selectedId"
      row-size="lg"
      @update:selected-id="select"
      @activate="runPrimary({ type: 'claudeItem', id: $event.id, item: $event })"
      @context="openPanel"
    >
      <template #row="{ item, selected: isSelected }">
        <ItemRow :item="item" :selected="isSelected" />
      </template>
    </ItemList>

    <template #pane>
      <ItemDetail v-if="selected" :item="selected" />
    </template>
  </ScreenFrame>
</template>
