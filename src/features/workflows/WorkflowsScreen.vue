<script setup lang="ts">
// The Workflows screen: a strip of three tabs (Sessions, Jobs, Idle) and the tab that is open.
// Each tab is its own view with its own list and pane; this screen only hands the open tab to the shell.
import { computed, useTemplateRef } from 'vue'
import ViewStrip, { type ViewStripItem } from '@/components/rain/ViewStrip.vue'
import { useScreen } from '@/composables/useScreen'
import { useNav } from '@/nav/useNav'
import IdleView from './idle/IdleView.vue'
import JobsView from './jobs/JobsView.vue'
import { useIdleJobs, useJobs, useSessions } from './resources'
import SessionsView from './sessions/SessionsView.vue'
import type { ViewController } from './viewController'

const SUB_VIEWS = ['sessions', 'jobs', 'idle'] as const
type SubView = (typeof SUB_VIEWS)[number]

const views = { sessions: SessionsView, jobs: JobsView, idle: IdleView }

const nav = useNav()
const sessions = useSessions()
const jobs = useJobs()
const idleJobs = useIdleJobs()

const current = computed<SubView>({
  get: () => SUB_VIEWS.find((id) => id === nav.route.value.sub) ?? 'sessions',
  set: (sub) => nav.setSub(sub),
})

const waiting = computed(() => sessions.data.value?.filter((session) => session.status === 'waiting').length ?? 0)

/** The Sessions count is the number that waits on you; with none waiting it is all sessions. */
const sessionCount = computed(() => (waiting.value > 0 ? waiting.value : sessions.data.value?.length))

const stripItems = computed<ViewStripItem[]>(() => [
  { id: 'sessions', label: 'Sessions', icon: 'bot', count: sessionCount.value, warn: waiting.value > 0 },
  { id: 'jobs', label: 'Jobs', icon: 'calendar-clock', count: jobs.data.value?.length },
  { id: 'idle', label: 'Idle', icon: 'cpu', count: idleJobs.data.value?.length },
])

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
  <component :is="views[current]" ref="view">
    <template #strip>
      <ViewStrip :items="stripItems" v-model="current" />
    </template>
  </component>
</template>
