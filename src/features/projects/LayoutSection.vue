<script setup lang="ts">
// The "Layout" section of the project's detail pane: a small diagram of the screen with the icon of
// what goes into each zone, and the way to the editor. Mount it with `:key="project.id"`: the
// layout is read once for each project and again when the project list changes.
import { watch } from 'vue'
import KeyHint from '@/components/rain/KeyHint.vue'
import SectionHeader from '@/components/rain/SectionHeader.vue'
import { Button } from '@/components/ui/button'
import { useResource } from '@/composables/useResource'
import { api } from '@/data'
import type { Project } from '@/data/types'
import { keepFocus } from '@/lib/focus'
import { layoutEditor } from './actions'
import LayoutDiagram from './LayoutDiagram.vue'

const props = defineProps<{ project: Project }>()

const layout = useResource(`projectLayout:${props.project.id}`, () => api.getProjectLayout(props.project.id))
const apps = useResource('apps', () => api.listApps())

// A hand edit of projects.toml or a save reaches the screen as a new project list.
watch(
  () => props.project.environment,
  () => void layout.reload(),
)
</script>

<template>
  <div class="mt-3">
    <div class="flex items-center">
      <SectionHeader class="-ml-2 flex-1" title="Layout" />
      <Button
        variant="ghost"
        size="sm"
        class="gap-2 text-13"
        tabindex="-1"
        @mousedown="keepFocus"
        @click="layoutEditor.edit(project)"
      >
        Edit
        <KeyHint keys="Ctrl+KeyE" />
      </Button>
    </div>

    <template v-if="layout.data.value">
      <LayoutDiagram
        v-if="layout.data.value.items.length > 0"
        :items="layout.data.value.items"
        :steps="layout.data.value.steps"
        :apps="apps.data.value ?? []"
      />
      <p v-else class="text-13 text-muted-foreground">Windows puts every window where it likes.</p>
      <p v-if="layout.data.value.scope === 'default'" class="mt-1 text-xs text-muted-foreground">
        The same for all projects.
      </p>
    </template>
  </div>
</template>
