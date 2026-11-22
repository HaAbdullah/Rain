<script setup lang="ts">
// The Idle tab: the machine's idle state above a list of jobs that run when the PC is idle.
import { MonitorIcon } from '@lucide/vue'
import { computed } from 'vue'
import type { ActionTarget } from '@/actions/types'
import EmptyState from '@/components/rain/EmptyState.vue'
import ErrorRow from '@/components/rain/ErrorRow.vue'
import ItemList from '@/components/rain/ItemList.vue'
import ScreenFrame from '@/components/rain/ScreenFrame.vue'
import { useActionPanel } from '@/composables/useActionPanel'
import { useSectionQuery } from '@/composables/useSearch'
import { useSelection, type SelGroup } from '@/composables/useSelection'
import type { IdleJob } from '@/data/types'
import { matches } from '@/lib/score'
import ListSkeleton from '../ListSkeleton.vue'
import { useIdleJobs, useMachine } from '../resources'
import { useRequestedItem } from '../useRequestedItem'
import { useRunAction } from '../useRunAction'
import IdleJobDetail from './IdleJobDetail.vue'
import IdleRow from './IdleRow.vue'

defineSlots<{ strip(): unknown }>()

/** Shown when no job says how long the PC must be idle. */
const DEFAULT_IDLE_AFTER_MINUTES = 10

const idleJobs = useIdleJobs()
const machine = useMachine()
const query = useSectionQuery('workflows')
const run = useRunAction()
const panel = useActionPanel()

const groups = computed<SelGroup<IdleJob>[]>(() => [
  { id: 'idle', items: (idleJobs.data.value ?? []).filter((job) => matches(query.value, job.name)) },
])

const hasData = computed(() => idleJobs.data.value !== undefined)
const shownCount = computed(() => groups.value[0].items.length)
const showSkeleton = computed(() => !hasData.value && idleJobs.showSkeleton.value)

const statusLine = computed(() => {
  const state = machine.data.value
  if (!state) return machine.error.value ? 'Machine state unavailable' : ''
  if (!state.userActive) return `PC idle for ${state.idleMinutes} min. Idle jobs can run.`
  const waits = (idleJobs.data.value ?? []).map((job) => job.idleAfterMinutes)
  const after = waits.length > 0 ? Math.min(...waits) : DEFAULT_IDLE_AFTER_MINUTES
  return `PC active. Idle jobs start after ${after} min idle.`
})

const { selectedId, selected, move, page, group, select } = useSelection(groups, {
  resetOn: query,
  persistKey: 'workflows.idle',
})
useRequestedItem(() => groups.value[0].items.map((job) => job.id), select)

const target = computed<ActionTarget | null>(() =>
  selected.value ? { type: 'idleJob', id: selected.value.id, item: selected.value } : null,
)

function openPanel(job: IdleJob, event: MouseEvent) {
  select(job.id)
  if (target.value) panel.open(target.value, event)
}

defineExpose({ target, move, page, group })
</script>

<template>
  <ScreenFrame pane="wide">
    <template #strip><slot name="strip" /></template>

    <ErrorRow v-if="idleJobs.error.value" class="m-2" message="Idle jobs unavailable" @retry="idleJobs.reload" />
    <ListSkeleton v-if="showSkeleton" size="lg" :rows="4" />
    <EmptyState v-else-if="hasData && idleJobs.data.value?.length === 0" icon="cpu" title="No idle jobs" />
    <EmptyState v-else-if="hasData && shownCount === 0" icon="search-x" title="No match" />
    <template v-else-if="hasData">
      <div class="flex h-10 shrink-0 items-center gap-2 border-b border-border px-4 text-13 text-muted-foreground">
        <MonitorIcon :size="16" :stroke-width="1.75" class="shrink-0" aria-hidden="true" />
        <span class="truncate">{{ statusLine }}</span>
      </div>
      <ItemList
        :groups="groups"
        :selected-id="selectedId"
        row-size="lg"
        @update:selected-id="select"
        @activate="run('log')"
        @context="openPanel"
      >
        <template #row="{ item, selected: isSelected }">
          <IdleRow :job="item" :selected="isSelected" />
        </template>
      </ItemList>
    </template>

    <template #pane>
      <IdleJobDetail v-if="selected" :job="selected" />
    </template>
  </ScreenFrame>
</template>
