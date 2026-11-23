<script setup lang="ts">
// Projects (spec 7.3): the most active projects as large cards above all the others,
// one selection across both grids, and the selected project's detail in the pane.
import { useWindowSize } from '@vueuse/core'
import { computed } from 'vue'
import { runPrimary } from '@/actions/context'
import type { ActionTarget } from '@/actions/types'
import EmptyState from '@/components/rain/EmptyState.vue'
import ErrorRow from '@/components/rain/ErrorRow.vue'
import ItemGrid from '@/components/rain/ItemGrid.vue'
import ScreenFrame from '@/components/rain/ScreenFrame.vue'
import SectionHeader from '@/components/rain/SectionHeader.vue'
import SourceNote from '@/components/rain/SourceNote.vue'
import { Skeleton } from '@/components/ui/skeleton'
import { useActionPanel } from '@/composables/useActionPanel'
import { useLaunch } from '@/composables/useLaunch'
import { useNow } from '@/composables/useNow'
import { useResource } from '@/composables/useResource'
import { useSourceNotes } from '@/composables/useSourceNotes'
import { useScreen } from '@/composables/useScreen'
import { useSectionQuery } from '@/composables/useSearch'
import { useSelection, type SelGroup } from '@/composables/useSelection'
import { api } from '@/data'
import type { Project } from '@/data/types'
import { matches } from '@/lib/score'
import { useNav } from '@/nav/useNav'
import { byLastOpened } from './project-order'
import { leadStatus, sessionsOfProject } from './project-sessions'
import ProjectCard from './ProjectCard.vue'
import ProjectDetail from './ProjectDetail.vue'

const ACTIVE_COUNT = 3
const ACTIVE_COLUMNS = 3
/** Below this window width a fifth column cuts the project names to a few letters. */
const WIDE_WINDOW = 1200

const nav = useNav()
const { width: windowWidth } = useWindowSize()
const allColumns = computed(() => (windowWidth.value < WIDE_WINDOW ? 4 : 5))
const query = useSectionQuery('projects')
const { runs } = useLaunch()
const now = useNow()
const panel = useActionPanel()
const projects = useResource('projects', () => api.listProjects(), { live: 'projects' })
// Git facts may be missing (git not found); the list itself still works.
const notes = useSourceNotes('git', 'projects')
const sessions = useResource('sessions', () => api.listSessions(), { live: 'sessions' })

const visible = computed(() =>
  (projects.data.value ?? []).filter((project) =>
    matches(query.value, project.name, [project.branch, project.path]),
  ),
)

// A project is in one group only: the selection finds items by id.
const groups = computed<SelGroup<Project>[]>(() => {
  const active = [...visible.value]
    .sort((a, b) => b.activityScore - a.activityScore)
    .slice(0, ACTIVE_COUNT)
  const others = visible.value
    .filter((project) => !active.includes(project))
    .sort(byLastOpened)
  return [
    { id: 'active', title: 'Most active', count: active.length, items: active, columns: ACTIVE_COLUMNS },
    { id: 'all', title: 'All projects', count: others.length, items: others, columns: allColumns.value },
  ]
})

const { selectedId, selected, move, page, group, select } = useSelection(groups, {
  resetOn: query,
  persistKey: 'projects',
})

function targetOf(project: Project): ActionTarget {
  return { type: 'project', id: project.id, item: project }
}

useScreen({
  target: computed(() => (selected.value ? targetOf(selected.value) : null)),
  move,
  page,
  group,
})

function sessionsOf(project: Project) {
  return sessionsOfProject(project, sessions.data.value ?? [])
}

function openPanel(project: Project, event: MouseEvent) {
  select(project.id)
  panel.open(targetOf(project), event)
}

const isEmpty = computed(() => projects.data.value?.length === 0)
const noMatch = computed(() => projects.data.value !== undefined && !isEmpty.value && visible.value.length === 0)
</script>

<template>
  <ScreenFrame pane="md">
    <div v-if="projects.error.value" class="px-4 pt-4">
      <ErrorRow message="Projects unavailable" :detail="projects.error.value.message" @retry="projects.reload()" />
    </div>

    <div v-if="notes.length > 0" class="px-4 pt-2">
      <SourceNote v-for="source in notes" :key="source.name" :source="source" />
    </div>

    <div v-if="projects.showSkeleton.value" class="flex flex-col gap-4 p-4" aria-hidden="true">
      <div>
        <SectionHeader class="-ml-2" title="Most active" />
        <div class="grid grid-cols-3 gap-3">
          <Skeleton v-for="n in ACTIVE_COLUMNS" :key="n" class="aspect-video rounded-lg" />
        </div>
      </div>
      <div>
        <SectionHeader class="-ml-2" title="All projects" />
        <div class="grid gap-3" :style="{ gridTemplateColumns: `repeat(${allColumns}, minmax(0, 1fr))` }">
          <Skeleton v-for="n in allColumns * 2" :key="n" class="h-22 rounded-lg" />
        </div>
      </div>
    </div>

    <EmptyState
      v-else-if="isEmpty"
      icon="folder-git-2"
      title="No projects found"
      hint="Add a project folder in Settings"
      action-label="Open settings"
      @action="nav.go('settings')"
    />
    <EmptyState v-else-if="noMatch" icon="search-x" title="No match" />

    <ItemGrid
      v-else-if="projects.data.value"
      :groups="groups"
      :selected-id="selectedId"
      @update:selected-id="select"
      @activate="runPrimary(targetOf($event))"
      @context="openPanel"
    >
      <template #tile="{ item, selected: isSelected }">
        <ProjectCard
          :project="item"
          :selected="isSelected"
          :size="groups[0].items.includes(item) ? 'lg' : 'sm'"
          :session-status="leadStatus(sessionsOf(item), now)"
        />
      </template>
    </ItemGrid>

    <template #pane>
      <ProjectDetail
        v-if="selected"
        :project="selected"
        :sessions="sessionsOf(selected)"
        :run="runs[selected.id]"
      />
      <EmptyState v-else icon="folder-git-2" title="No project selected" />
    </template>
  </ScreenFrame>
</template>
