<script setup lang="ts">
// Home and launcher (spec 7.1). Without a query: Needs you, Pinned, Projects and
// Recent, at most 14 rows. With a query: one flat ranked list of search items and
// commands. TodayPanel sits in the pane and is not part of the cursor.
import { computed, nextTick, onScopeDispose, shallowRef, watch, type Ref } from 'vue'
import { commands } from '@/actions/commands'
import { runPrimary } from '@/actions/context'
import { useTargetResolver } from '@/actions/targets'
import EmptyState from '@/components/rain/EmptyState.vue'
import ErrorRow from '@/components/rain/ErrorRow.vue'
import ItemList from '@/components/rain/ItemList.vue'
import ScreenFrame from '@/components/rain/ScreenFrame.vue'
import { Skeleton } from '@/components/ui/skeleton'
import { useActionPanel } from '@/composables/useActionPanel'
import { useLaunch } from '@/composables/useLaunch'
import { useResource } from '@/composables/useResource'
import { useScreen } from '@/composables/useScreen'
import { useSearch } from '@/composables/useSearch'
import { useSelection, type SelGroup } from '@/composables/useSelection'
import { api } from '@/data'
import type { RainError } from '@/data/api'
import type { SearchItem, Status } from '@/data/types'
import { score } from '@/lib/score'
import { shell } from '@/shell'
import { commandRow, failedJobRow, itemRow, projectRow, sessionRow, type HomeRow } from './homeRows'
import ResultRow from './ResultRow.vue'
import TodayPanel from './TodayPanel.vue'

const NEEDS_YOU_CAP = 3
const PINNED_CAP = 5
const PROJECTS_CAP = 3
const RECENT_CAP = 3
/** More than RECENT_CAP, because recent items that are already listed above are dropped. */
const RECENT_FETCH = 12
const MAX_RESULTS = 50
/** Alt+1..9 run the first nine rows. */
const DIGIT_ROWS = 9
const SKELETON_ROWS = 8

const { query } = useSearch()
const { runs } = useLaunch()
const resolve = useTargetResolver()

// The same keys the target resolver uses, so every list is fetched once and shared.
const apps = useResource('apps', () => api.listApps())
const pools = useResource('pools', () => api.listPools())
const projects = useResource('projects', () => api.listProjects(), { live: 'projects' })
const games = useResource('games', () => api.listGames())
const sessions = useResource('sessions', () => api.listSessions(), { live: 'sessions' })
const jobs = useResource('jobs', () => api.listJobs(), { live: 'jobs' })
const idleJobs = useResource('idleJobs', () => api.listIdleJobs(), { live: 'idleJobs' })
const pinned = useResource('pinned', () => api.listPinned())
const recent = useResource('recent', () => api.listRecent(RECENT_FETCH))

interface Source {
  label: string
  resource: {
    data: Ref<unknown>
    error: Ref<RainError | null>
    showSkeleton: Ref<boolean>
    reload(): Promise<void>
  }
}

const sources: Source[] = [
  { label: 'Apps', resource: apps },
  { label: 'Pools', resource: pools },
  { label: 'Projects', resource: projects },
  { label: 'Games', resource: games },
  { label: 'Sessions', resource: sessions },
  { label: 'Jobs', resource: jobs },
  { label: 'Idle jobs', resource: idleJobs },
  { label: 'Pinned items', resource: pinned },
  { label: 'Recent items', resource: recent },
]

// Rows are built only when every list has answered, so the cursor never starts on
// a row that a later answer would push down.
const ready = computed(() =>
  sources.every(({ resource }) => resource.data.value !== undefined || resource.error.value !== null),
)
const showSkeleton = computed(() => !ready.value && sources.some(({ resource }) => resource.showSkeleton.value))

const appList = computed(() => apps.data.value ?? [])

/** A row for a search item, or null while the list that holds its entity is still loading. */
function rowOf(item: SearchItem): HomeRow | null {
  const target = resolve(item)
  return target ? itemRow(item, target, appList.value) : null
}

function rowsOf(items: SearchItem[] | undefined): HomeRow[] {
  return (items ?? []).map(rowOf).filter((row) => row !== null)
}

// ---- with a query: one flat ranked list ----

/** The api's answer together with the query it answers. */
const found = shallowRef<{ query: string; items: SearchItem[]; failed: boolean }>({
  query: '',
  items: [],
  failed: false,
})
let newestSearch = 0

async function runSearch(text: string) {
  const mine = ++newestSearch
  let answer = { query: text, items: [] as SearchItem[], failed: false }
  if (text !== '') {
    try {
      answer = { query: text, items: await api.search(text), failed: false }
    } catch {
      answer = { query: text, items: [], failed: true }
    }
  }
  // An answer to an older query must not replace a newer one.
  if (mine === newestSearch) found.value = answer
}

watch(query, runSearch, { immediate: true })

/** True until the api has answered for the text now in the field. The old rows stay drawn meanwhile. */
const pending = computed(() => query.value !== '' && found.value.query !== query.value)

const resultRows = computed<HomeRow[]>(() => {
  if (query.value === '') return []
  const fromItems = found.value.items.flatMap((item) => {
    const row = rowOf(item)
    return row ? [{ row, score: item.score }] : []
  })
  const fromCommands = commands.flatMap((command) => {
    const commandScore = score(query.value, command.title, command.keywords)
    return commandScore === 0 ? [] : [{ row: commandRow(command), score: commandScore }]
  })
  return [...fromItems, ...fromCommands]
    .sort((a, b) => b.score - a.score)
    .slice(0, MAX_RESULTS)
    .map((entry) => entry.row)
})

// ---- without a query: Needs you, Pinned, Projects, Recent ----

const needsYouRows = computed(() => [
  ...(sessions.data.value ?? []).filter((session) => session.status === 'waiting').map(sessionRow),
  ...(jobs.data.value ?? []).filter((job) => job.recentRuns[0]?.status === 'failed').map(failedJobRow),
])

const launcherGroups = computed<SelGroup<HomeRow>[]>(() => {
  if (!ready.value) return []
  // Pinned pools first, then the other pinned items; the sort keeps pin order inside each part.
  const allPinned = rowsOf(pinned.data.value).sort(
    (a, b) => Number(b.target.type === 'pool') - Number(a.target.type === 'pool'),
  )
  const pinnedIds = new Set(allPinned.map((row) => row.id))
  const projectRows = [...(projects.data.value ?? [])]
    .sort((a, b) => b.activityScore - a.activityScore)
    .map(projectRow)
    .filter((row) => !pinnedIds.has(row.id))
    .slice(0, PROJECTS_CAP)
  const listed = new Set([...allPinned.slice(0, PINNED_CAP), ...projectRows].map((row) => row.id))
  const recentRows = rowsOf(recent.data.value)
    .filter((row) => !listed.has(row.id))
    .slice(0, RECENT_CAP)
  return [
    {
      id: 'needs-you',
      title: 'Needs you',
      count: needsYouRows.value.length,
      items: needsYouRows.value.slice(0, NEEDS_YOU_CAP),
    },
    { id: 'pinned', title: 'Pinned', items: allPinned.slice(0, PINNED_CAP) },
    { id: 'projects', title: 'Projects', items: projectRows },
    { id: 'recent', title: 'Recent', items: recentRows },
  ]
})

const groups = computed<SelGroup<HomeRow>[]>(() =>
  query.value === '' ? launcherGroups.value : [{ id: 'results', items: resultRows.value }],
)

const rows = computed(() => groups.value.flatMap((group) => group.items))

/** The row the cursor starts on: the first Pinned row, or the first result. Needs you is one Up away. */
const startRow = computed(
  () => rows.value.find((row) => !row.needsYou || query.value !== '') ?? rows.value[0],
)

/** Rows that answer to Alt+1..9. Needs you rows have no digit. */
const digitRows = computed(() => rows.value.filter((row) => !row.needsYou).slice(0, DIGIT_ROWS))
const digitOf = computed(() => new Map(digitRows.value.map((row, i) => [row.id, i + 1])))

/** Two results with the same title need their subtitles to tell them apart. */
const collidingTitles = computed(() => {
  const seen = new Set<string>()
  const colliding = new Set<string>()
  for (const row of resultRows.value) {
    const key = row.title.toLowerCase()
    if (seen.has(key)) colliding.add(key)
    seen.add(key)
  }
  return colliding
})

function subtitleOf(row: HomeRow): string | undefined {
  if (query.value === '') return row.subtitle
  return collidingTitles.value.has(row.title.toLowerCase()) ? (row.disambiguator ?? row.subtitle) : undefined
}

// ---- selection ----

const { selectedId, selected, move, page, group, select } = useSelection(groups, {
  initial: () => startRow.value?.id,
})

// A new list (another query, or back to none) puts the cursor on its start row.
// The key is undefined while an answer is on its way, so a late answer selects once.
const listKey = computed(() => (pending.value ? undefined : query.value))
watch(listKey, (key) => {
  if (key !== undefined && startRow.value) select(startRow.value.id)
})

// Every summon starts on the first Pinned row, wherever the cursor was when Rain hid.
const stopSummonWatch = shell.onSummon(() => {
  void nextTick(() => {
    if (startRow.value) select(startRow.value.id)
  })
})
onScopeDispose(stopSummonWatch)

useScreen({
  // Rows of an older query are still drawn while pending; Enter must not run one of them.
  target: computed(() => (pending.value ? null : (selected.value?.target ?? null))),
  move,
  page,
  group,
  digits: computed(() => (pending.value ? [] : digitRows.value.map((row) => row.target))),
})

// ---- errors ----

// A failed list drops its items from the results too, so it is named with a query as well.
const failure = computed(() => {
  const failed = sources.filter(({ resource }) => resource.error.value !== null)
  const searchFailed = query.value !== '' && found.value.failed
  const labels = [...(searchFailed ? ['Search'] : []), ...failed.map(({ label }) => label)]
  if (labels.length === 0) return null
  return {
    message: `${labels.join(', ')} unavailable`,
    // The backend's text for the first list that failed (a search failure carries none).
    detail: failed[0]?.resource.error.value?.message,
    retry: () =>
      Promise.all([
        ...(searchFailed ? [runSearch(query.value)] : []),
        ...failed.map(({ resource }) => resource.reload()),
      ]),
  }
})

// ---- drawing ----

/** A launch in progress or just failed shows on its row; otherwise the item's own status. */
function statusOf(row: HomeRow): Status | undefined {
  const run = runs.value[row.target.id]
  return run && run.status !== 'success' ? run.status : row.status
}

function openPanel(row: HomeRow, event: MouseEvent) {
  select(row.id)
  useActionPanel().open(row.target, event)
}
</script>

<template>
  <ScreenFrame pane="md">
    <div v-if="showSkeleton" class="flex flex-col p-2" aria-hidden="true">
      <div v-for="n in SKELETON_ROWS" :key="n" class="flex h-(--row-h) items-center gap-3 px-2">
        <Skeleton class="size-5 shrink-0" />
        <Skeleton class="h-3.5 w-1/3" />
      </div>
    </div>

    <template v-else>
      <ErrorRow
        v-if="failure"
        class="mx-2 mt-2"
        :message="failure.message"
        :detail="failure.detail"
        @retry="failure.retry()"
      />
      <EmptyState v-if="rows.length === 0 && query !== '' && !pending && ready" icon="search-x" title="No match" />
      <EmptyState
        v-else-if="rows.length === 0 && query === '' && ready"
        icon="house"
        title="Nothing pinned yet"
        hint="Type to search apps, projects, games and commands."
      />
      <ItemList
        v-else-if="rows.length > 0"
        :groups="groups"
        :selected-id="selectedId"
        click-runs
        @update:selected-id="select"
        @activate="runPrimary($event.target)"
        @context="openPanel"
      >
        <template #row="{ item, selected: isSelected }">
          <ResultRow
            :row="item"
            :selected="isSelected"
            :status="statusOf(item)"
            :digit="digitOf.get(item.id)"
            :subtitle="subtitleOf(item)"
          />
        </template>
      </ItemList>
    </template>

    <template #pane>
      <TodayPanel />
    </template>
  </ScreenFrame>
</template>
