<script setup lang="ts">
// The Claude screen: what the Claude setup of the owner contains, read only. A strip of five
// sub-views (Skills, Memory, Instructions, Agents, Setup). Each list view has its own list and
// pane; this screen hands the open view to the shell.
import { computed, useTemplateRef } from 'vue'
import ViewStrip, { type ViewStripItem } from '@/components/rain/ViewStrip.vue'
import { useScreen } from '@/composables/useScreen'
import { useNav } from '@/nav/useNav'
import LibraryView from './LibraryView.vue'
import { countsByView, setupCount, SUB_VIEWS, VIEW_ICON, VIEW_LABEL, type ListView, type SubView } from './model'
import { useClaudeLibrary } from './resources'
import SetupView from './SetupView.vue'
import type { ViewController } from './viewController'

const nav = useNav()
const library = useClaudeLibrary()

const current = computed<SubView>({
  get: () => SUB_VIEWS.find((id) => id === nav.route.value.sub) ?? 'skills',
  set: (sub) => nav.setSub(sub),
})

/** The view as a list view, or null for Setup. */
const listView = computed<ListView | null>(() => (current.value === 'setup' ? null : current.value))

const stripItems = computed<ViewStripItem[]>(() => {
  const data = library.data.value
  const counts = data ? countsByView(data.items) : undefined
  return SUB_VIEWS.map((id) => ({
    id,
    label: VIEW_LABEL[id],
    icon: VIEW_ICON[id],
    count: id === 'setup' ? (data ? setupCount(data.setup) : undefined) : counts?.[id],
  }))
})

const view = useTemplateRef<ViewController>('view')

useScreen({
  target: computed(() => view.value?.target ?? null),
  move: (dir) => view.value?.move(dir) ?? false,
  page: (dir) => view.value?.page(dir),
  group: (dir) => view.value?.group(dir),
  subView: { ids: SUB_VIEWS, current },
})
</script>

<template>
  <LibraryView v-if="listView" :key="listView" ref="view" :view="listView">
    <template #strip>
      <ViewStrip v-model="current" :items="stripItems" />
    </template>
  </LibraryView>
  <SetupView v-else ref="view">
    <template #strip>
      <ViewStrip v-model="current" :items="stripItems" />
    </template>
  </SetupView>
</template>
