<script setup lang="ts">
// The detail pane of the selected project: where it is, its dev environment (a launch
// checklist while it starts), the Claude sessions in it, and the two open buttons.
import { FolderXIcon, GitBranchIcon } from '@lucide/vue'
import { computed, ref, watch } from 'vue'
import { runAction } from '@/actions/context'
import DetailPane from '@/components/rain/DetailPane.vue'
import KeyHint from '@/components/rain/KeyHint.vue'
import LaunchChecklist from '@/components/rain/LaunchChecklist.vue'
import PathText from '@/components/rain/PathText.vue'
import RIcon from '@/components/rain/RIcon.vue'
import SectionHeader from '@/components/rain/SectionHeader.vue'
import StatusDot from '@/components/rain/StatusDot.vue'
import { Button } from '@/components/ui/button'
import { useNow } from '@/composables/useNow'
import { useScreenState } from '@/composables/useScreen'
import type { ClaudeSession, LaunchRun, Project } from '@/data/types'
import { keepFocus } from '@/lib/focus'
import { useNav } from '@/nav/useNav'
import { sessionStatus } from './project-sessions'

const props = defineProps<{
  project: Project
  sessions: ClaudeSession[]
  /** The newest launch of this project, if any. */
  run?: LaunchRun
}>()

const { primary, secondary } = useScreenState()
const nav = useNav()
const now = useNow()

const changedFiles = computed(() => {
  const count = props.project.dirtyFiles
  if (count === 0) return 'No changed files'
  return count === 1 ? '1 changed file' : `${count} changed files`
})

// A finished launch hides Rain. A failed one stays only for the run that was seen starting
// while this project was selected, so an old failure does not come back on re-selection.
const watchedRunId = ref<string>()
watch(
  () => props.project.id,
  () => (watchedRunId.value = undefined),
)
watch(
  () => props.run,
  (run) => {
    if (run?.status === 'running') watchedRunId.value = run.id
  },
  { immediate: true },
)
const checklist = computed(() => {
  const run = props.run
  const shown = run?.status === 'running' || (run?.status === 'failed' && run.id === watchedRunId.value)
  return run && shown ? run.steps : []
})
const starting = computed(() => props.run?.status === 'running')
</script>

<template>
  <DetailPane :title="project.name" :icon="project.icon">
    <div class="flex flex-col gap-1">
      <PathText :path="project.path" />
      <div v-if="project.missing" class="mt-3 flex items-center gap-2 text-13 text-muted-foreground">
        <FolderXIcon :size="16" :stroke-width="1.75" class="shrink-0 text-status-failed" aria-hidden="true" />
        Folder not found. Check the project folders in Settings.
      </div>
      <template v-else>
        <div class="mt-2 flex items-center gap-2 text-muted-foreground">
          <GitBranchIcon :size="12" :stroke-width="1.75" class="shrink-0" aria-hidden="true" />
          <span class="selectable min-w-0 truncate font-mono text-xs">{{ project.branch }}</span>
        </div>
        <p class="text-13 text-muted-foreground">{{ changedFiles }}</p>
      </template>
    </div>

    <template v-if="!project.missing">
      <SectionHeader class="-ml-2 mt-3" title="Environment" />
      <LaunchChecklist v-if="checklist.length > 0" :steps="checklist" />
      <ul v-else class="flex flex-col">
        <li v-for="(step, index) in project.environment" :key="index" class="flex h-8 items-center gap-3">
          <RIcon :icon="step.icon" :size="16" />
          <span class="shrink-0 text-sm text-foreground">{{ step.label }}</span>
          <span
            class="min-w-0 flex-1 truncate text-right font-mono text-xs text-muted-foreground"
            :title="step.command ?? step.url"
          >
            {{ step.command ?? step.url }}
          </span>
        </li>
      </ul>
    </template>

    <template v-if="sessions.length > 0">
      <SectionHeader class="-ml-2 mt-3" title="Sessions here" :count="sessions.length" />
      <ul class="flex flex-col">
        <li v-for="session in sessions" :key="session.id">
          <button
            type="button"
            tabindex="-1"
            class="-mx-2 flex h-8 w-[calc(100%+1rem)] items-center gap-3 rounded-md px-2 text-left transition-colors hover:bg-hover"
            @mousedown="keepFocus"
            @click="nav.go('workflows', 'sessions', { id: session.id })"
          >
            <StatusDot :status="sessionStatus(session, now)" />
            <span class="min-w-0 flex-1 truncate text-sm text-foreground">{{ session.title }}</span>
          </button>
        </li>
      </ul>
    </template>

    <template #actions>
      <Button
        v-if="primary"
        tabindex="-1"
        class="gap-2"
        :disabled="starting"
        @mousedown="keepFocus"
        @click="runAction(primary)"
      >
        {{ primary.label }}
        <KeyHint keys="Enter" />
      </Button>
      <Button
        v-if="secondary"
        variant="outline"
        tabindex="-1"
        class="gap-2"
        :disabled="starting"
        @mousedown="keepFocus"
        @click="runAction(secondary)"
      >
        {{ secondary.label }}
        <KeyHint keys="Ctrl+Enter" />
      </Button>
    </template>
  </DetailPane>
</template>
