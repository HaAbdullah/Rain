<script setup lang="ts">
// The Apps screen (spec 7.2): pools and apps in one list, the selected one in the detail pane.
import { PinIcon } from '@lucide/vue'
import { computed, watch } from 'vue'
import { commands } from '@/actions/commands'
import { runAction, runPrimary } from '@/actions/context'
import type { ActionTarget } from '@/actions/types'
import EmptyState from '@/components/rain/EmptyState.vue'
import ErrorRow from '@/components/rain/ErrorRow.vue'
import IconStack from '@/components/rain/IconStack.vue'
import ItemList from '@/components/rain/ItemList.vue'
import KeyHint from '@/components/rain/KeyHint.vue'
import ListRow from '@/components/rain/ListRow.vue'
import ScreenFrame from '@/components/rain/ScreenFrame.vue'
import StatusDot from '@/components/rain/StatusDot.vue'
import { Skeleton } from '@/components/ui/skeleton'
import { useActionPanel } from '@/composables/useActionPanel'
import { useLaunch } from '@/composables/useLaunch'
import { useNotice } from '@/composables/useNotice'
import { useResource } from '@/composables/useResource'
import { useScreen, useScreenState } from '@/composables/useScreen'
import { useSectionQuery } from '@/composables/useSearch'
import { useSelection, type SelGroup } from '@/composables/useSelection'
import { api } from '@/data'
import type { AppEntry, IconRef, Pool, Status } from '@/data/types'
import { errorMessage } from '@/lib/errors'
import { matches } from '@/lib/score'
import { useNav } from '@/nav/useNav'
import { rescanApps } from './actions'
import AppDetail from './AppDetail.vue'
import PoolDetail from './PoolDetail.vue'
import PoolEditorDialog from './PoolEditorDialog.vue'

/** A list row. Pool and app ids may be equal, so the row id carries the kind. */
type Row =
  | { id: string; kind: 'pool'; pool: Pool }
  | { id: string; kind: 'app'; app: AppEntry }
  /** The dashed "New pool" row, shown only while there are no pools. */
  | { id: string; kind: 'new' }

const SKELETON_ROWS = 8
const NEW_POOL_ROW: Row = { id: 'new-pool', kind: 'new' }
const NEW_POOL_COMMAND = commands.find((command) => command.id === 'new-pool')

const nav = useNav()
const query = useSectionQuery('apps')
const { runs } = useLaunch()
const notice = useNotice()
const { primary, secondary } = useScreenState()
const apps = useResource('apps', () => api.listApps())
const pools = useResource('pools', () => api.listPools())

/** The empty state's button: read the installed apps again. */
async function scan(): Promise<void> {
  try {
    await rescanApps({ api, notice })
  } catch (reason) {
    notice.error(errorMessage(reason, 'Could not scan for apps'))
  }
}

const appList = computed(() => apps.data.value ?? [])
const poolList = computed(() => pools.data.value ?? [])
const appById = computed(() => new Map(appList.value.map((app) => [app.id, app])))

// ---- the list ----

const groups = computed<SelGroup<Row>[]>(() => {
  const poolRows: Row[] = poolList.value
    .filter((pool) => matches(query.value, pool.name))
    .map((pool) => ({ id: `pool:${pool.id}`, kind: 'pool', pool }))
  // With no pools at all (not merely none that match) the group offers to make one.
  if (pools.data.value?.length === 0 && query.value === '') poolRows.push(NEW_POOL_ROW)

  const appRows: Row[] = appList.value
    .filter((app) => matches(query.value, app.name, app.keywords))
    .sort((a, b) => a.name.localeCompare(b.name))
    .map((app) => ({ id: `app:${app.id}`, kind: 'app', app }))

  return [
    { id: 'pools', title: 'Pools', count: poolRows.length, items: poolRows },
    { id: 'apps', title: 'Apps', count: appRows.length, items: appRows },
  ]
})

const hasRows = computed(() => groups.value.some((group) => group.items.length > 0))

const { selectedId, selected, move, page, group, select } = useSelection(groups, {
  resetOn: query,
  persistKey: 'apps.selected',
})

// "Edit pool" from Home arrives with the pool's id: select it once the list has it.
let wantedPoolId = nav.route.value.params?.id
watch(
  groups,
  (current) => {
    if (!wantedPoolId) return
    const row = current.flatMap((entry) => entry.items).find((item) => item.kind === 'pool' && item.pool.id === wantedPoolId)
    if (!row) return
    select(row.id)
    wantedPoolId = undefined
  },
  { immediate: true },
)

// ---- what Enter, Ctrl+Enter and Ctrl+K act on ----

function targetOf(row: Row): ActionTarget | null {
  switch (row.kind) {
    case 'pool':
      return { type: 'pool', id: row.pool.id, item: row.pool }
    case 'app':
      return { type: 'app', id: row.app.id, item: row.app }
    case 'new':
      return NEW_POOL_COMMAND ? { type: 'command', id: NEW_POOL_COMMAND.id, item: NEW_POOL_COMMAND } : null
  }
}

useScreen({
  target: computed(() => (selected.value ? targetOf(selected.value) : null)),
  move,
  page,
  group,
})

function activate(row: Row) {
  const target = targetOf(row)
  if (target) runPrimary(target)
}

function openPanel(row: Row, event: MouseEvent) {
  const target = targetOf(row)
  if (!target) return
  select(row.id)
  useActionPanel().open(target, event)
}

/** The pane's buttons do what Enter and Ctrl+Enter do. */
function runSlot(action: typeof primary.value) {
  if (action) void runAction(action)
}

// ---- row details ----

/** A launch that is running or failed shows on the row; otherwise only what the app itself says. */
function launchStatus(id: string): Status | undefined {
  const run = runs.value[id]
  return run && run.status !== 'success' ? run.status : undefined
}

function memberIcons(pool: Pool): IconRef[] {
  return pool.members.flatMap((member) => appById.value.get(member.appId)?.icon ?? [])
}

// ---- states ----

const failedSources = computed(() => [
  { label: 'Apps', resource: apps },
  { label: 'Pools', resource: pools },
].filter((source) => source.resource.error.value !== null))

const loading = computed(
  () =>
    (apps.data.value === undefined && apps.error.value === null) ||
    (pools.data.value === undefined && pools.error.value === null),
)
const showSkeleton = computed(() => loading.value && (apps.showSkeleton.value || pools.showSkeleton.value))
const noApps = computed(() => apps.data.value?.length === 0 && pools.data.value?.length === 0)
</script>

<template>
  <ScreenFrame pane="md">
    <div v-if="failedSources.length > 0" class="shrink-0 px-2 pt-2">
      <ErrorRow
        v-for="source in failedSources"
        :key="source.label"
        :message="`${source.label} unavailable`"
        :detail="source.resource.error.value?.message"
        @retry="source.resource.reload()"
      />
    </div>

    <div v-if="loading" class="p-2" aria-hidden="true">
      <template v-if="showSkeleton">
        <div v-for="n in SKELETON_ROWS" :key="n" class="flex h-(--row-h) items-center gap-3 px-2">
          <Skeleton class="size-5" />
          <Skeleton class="h-3.5 w-40" />
        </div>
      </template>
    </div>
    <EmptyState
      v-else-if="noApps"
      icon="layout-grid"
      title="No apps found"
      hint="Rain lists the apps that Windows shows in the Start menu."
      action-label="Scan again"
      @action="scan"
    />
    <EmptyState v-else-if="!hasRows && query !== ''" icon="search-x" title="No match" />
    <ItemList
      v-else-if="hasRows"
      :groups="groups"
      :selected-id="selectedId"
      @update:selected-id="select"
      @activate="activate"
      @context="openPanel"
    >
      <template #row="{ item, selected: isSelected }">
        <div
          v-if="item.kind === 'new'"
          class="flex h-10 items-center gap-3 rounded-lg border border-dashed px-2 transition-colors"
          :class="isSelected ? 'border-border-strong bg-selected' : 'border-border hover:bg-hover'"
        >
          <span class="min-w-0 flex-1 truncate text-sm text-muted-foreground">New pool</span>
          <KeyHint keys="Ctrl+N" />
        </div>

        <ListRow
          v-else-if="item.kind === 'pool'"
          :icon="item.pool.icon"
          :title="item.pool.name"
          :status="launchStatus(item.pool.id)"
          :selected="isSelected"
        >
          <template #trailing>
            <IconStack :icons="memberIcons(item.pool)" :max="4" />
            <PinIcon v-if="item.pool.pinned" :size="12" aria-label="Pinned" />
          </template>
        </ListRow>

        <ListRow
          v-else
          :icon="item.app.icon"
          :title="item.app.name"
          :status="launchStatus(item.app.id)"
          :selected="isSelected"
        >
          <template #trailing>
            <StatusDot v-if="item.app.running" status="success" :size="12" label="Running" />
            <PinIcon v-if="item.app.pinned" :size="12" aria-label="Pinned" />
          </template>
        </ListRow>
      </template>
    </ItemList>

    <PoolEditorDialog />

    <template #pane>
      <PoolDetail
        v-if="selected?.kind === 'pool'"
        :pool="selected.pool"
        :apps="appList"
        @launch="runSlot(primary)"
        @edit="runSlot(secondary)"
      />
      <AppDetail
        v-else-if="selected?.kind === 'app'"
        :app="selected.app"
        :pools="poolList"
        @launch="runSlot(primary)"
        @reveal="runSlot(secondary)"
      />
    </template>
  </ScreenFrame>
</template>
