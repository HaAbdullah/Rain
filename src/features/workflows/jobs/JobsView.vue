<script setup lang="ts">
// The Jobs tab: scheduled jobs under a column header row, and the selected job in the pane.
import { computed } from 'vue'
import type { ActionTarget } from '@/actions/types'
import EmptyState from '@/components/rain/EmptyState.vue'
import ErrorRow from '@/components/rain/ErrorRow.vue'
import ItemList from '@/components/rain/ItemList.vue'
import ScreenFrame from '@/components/rain/ScreenFrame.vue'
import { useActionPanel } from '@/composables/useActionPanel'
import { useSectionQuery } from '@/composables/useSearch'
import { useSelection, type SelGroup } from '@/composables/useSelection'
import type { ScheduledJob } from '@/data/types'
import { matches } from '@/lib/score'
import ListSkeleton from '../ListSkeleton.vue'
import { useJobs } from '../resources'
import { useRequestedItem } from '../useRequestedItem'
import { useRunAction } from '../useRunAction'
import { JOB_GRID } from './grid'
import JobDetail from './JobDetail.vue'
import JobRow from './JobRow.vue'

defineSlots<{ strip(): unknown }>()

const jobs = useJobs()
const query = useSectionQuery('workflows')
const run = useRunAction()
const panel = useActionPanel()

/** Last run failed, then running, then the others; a disabled job goes last. */
function rank(job: ScheduledJob): number {
  if (!job.enabled) return 3
  if (job.recentRuns[0]?.status === 'failed') return 0
  return job.running ? 1 : 2
}

function nextRunTime(job: ScheduledJob): number {
  return job.nextRunAt ? Date.parse(job.nextRunAt) : Infinity
}

function byOrder(a: ScheduledJob, b: ScheduledJob): number {
  return rank(a) - rank(b) || nextRunTime(a) - nextRunTime(b)
}

const groups = computed<SelGroup<ScheduledJob>[]>(() => {
  const visible = (jobs.data.value ?? [])
    .filter((job) => matches(query.value, job.name, [job.scheduleLabel, job.cron]))
    .sort(byOrder)
  return [{ id: 'jobs', items: visible }]
})

const hasData = computed(() => jobs.data.value !== undefined)
const shownCount = computed(() => groups.value[0].items.length)
const showSkeleton = computed(() => !hasData.value && jobs.showSkeleton.value)

const { selectedId, selected, move, page, group, select } = useSelection(groups, {
  resetOn: query,
  persistKey: 'workflows.jobs',
})
useRequestedItem(() => groups.value[0].items.map((job) => job.id), select)

const target = computed<ActionTarget | null>(() =>
  selected.value ? { type: 'job', id: selected.value.id, item: selected.value } : null,
)

function openPanel(job: ScheduledJob, event: MouseEvent) {
  select(job.id)
  if (target.value) panel.open(target.value, event)
}

defineExpose({ target, move, page, group })
</script>

<template>
  <ScreenFrame pane="wide">
    <template #strip><slot name="strip" /></template>

    <ErrorRow v-if="jobs.error.value" class="m-2" message="Jobs unavailable" :detail="jobs.error.value.message" @retry="jobs.reload" />
    <ListSkeleton v-if="showSkeleton" />
    <EmptyState
      v-else-if="hasData && jobs.data.value?.length === 0"
      icon="calendar-clock"
      title="No scheduled jobs"
      hint="Jobs are defined in jobs.toml in your data folder. Scheduled tasks of the Claude app show here too"
    />
    <EmptyState v-else-if="hasData && shownCount === 0" icon="search-x" title="No match" />
    <template v-else-if="hasData">
      <!-- px-4 lines the headings up with the rows: 8 px list padding plus 8 px row padding. -->
      <div class="h-7 shrink-0 px-4 text-xs text-muted-foreground" :class="JOB_GRID">
        <span />
        <span>Name</span>
        <span>Schedule</span>
        <span>Next</span>
        <span>Last 10</span>
        <span />
      </div>
      <ItemList
        :groups="groups"
        :selected-id="selectedId"
        @update:selected-id="select"
        @activate="selected?.source === 'claude-app' ? undefined : run('log')"
        @context="openPanel"
      >
        <template #row="{ item, selected: isSelected }">
          <JobRow :job="item" :selected="isSelected" />
        </template>
      </ItemList>
    </template>

    <template #pane>
      <JobDetail v-if="selected" :job="selected" />
    </template>
  </ScreenFrame>
</template>
