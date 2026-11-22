<script setup lang="ts">
// The Sessions tab: Claude sessions with the ones that wait on you first, and the selected one in the pane.
import { computed, watch } from 'vue'
import type { ActionTarget } from '@/actions/types'
import EmptyState from '@/components/rain/EmptyState.vue'
import ErrorRow from '@/components/rain/ErrorRow.vue'
import ItemList from '@/components/rain/ItemList.vue'
import ScreenFrame from '@/components/rain/ScreenFrame.vue'
import { useActionPanel } from '@/composables/useActionPanel'
import { useNow } from '@/composables/useNow'
import { useSectionQuery } from '@/composables/useSearch'
import { useSelection, type SelGroup } from '@/composables/useSelection'
import type { ClaudeSession, Status } from '@/data/types'
import { matches } from '@/lib/score'
import { answering } from '../actions'
import ListSkeleton from '../ListSkeleton.vue'
import { useRequestedItem } from '../useRequestedItem'
import { useRunAction } from '../useRunAction'
import { useSessions } from '../resources'
import SessionDetail from './SessionDetail.vue'
import SessionRow from './SessionRow.vue'
import { folderName, silentMinutes } from './sessionInfo'

defineSlots<{ strip(): unknown }>()

/** The groups, in this fixed order. They never collapse and are left out when empty. */
const GROUPS: { status: Status; title: string }[] = [
  { status: 'waiting', title: 'Waiting on you' },
  { status: 'failed', title: 'Failed' },
  { status: 'running', title: 'Working' },
  { status: 'success', title: 'Done' },
  { status: 'idle', title: 'Idle' },
]

const sessions = useSessions()
const query = useSectionQuery('workflows')
const now = useNow()
const run = useRunAction()
const panel = useActionPanel()

/** The session that has waited longest comes first; the others by latest activity. */
function byUrgency(a: ClaudeSession, b: ClaudeSession): number {
  if (a.status === 'waiting' && a.request && b.request) {
    return Date.parse(a.request.requestedAt) - Date.parse(b.request.requestedAt)
  }
  return Date.parse(b.lastActivityAt) - Date.parse(a.lastActivityAt)
}

const groups = computed<SelGroup<ClaudeSession>[]>(() => {
  const visible = (sessions.data.value ?? []).filter((session) =>
    matches(query.value, `${session.title} ${folderName(session.cwd)}`, [
      session.cwd,
      session.branch ?? '',
      session.request?.summary ?? '',
    ]),
  )
  return GROUPS.map(({ status, title }) => {
    const items = visible.filter((session) => session.status === status).sort(byUrgency)
    return { id: status, title, count: items.length, items }
  })
})

const hasData = computed(() => sessions.data.value !== undefined)
const shownCount = computed(() => groups.value.reduce((sum, group) => sum + group.items.length, 0))
const showSkeleton = computed(() => !hasData.value && sessions.showSkeleton.value)

const { selectedId, selected, move, page, group, select } = useSelection(groups, { resetOn: query })
useRequestedItem(() => groups.value.flatMap((g) => g.items.map((s) => s.id)), select)

const target = computed<ActionTarget | null>(() =>
  selected.value ? { type: 'session', id: selected.value.id, item: selected.value } : null,
)

// As soon as the selected session is answered, move on to the next one that still waits,
// so quick repeated answers each reach a new session.
watch(
  () => selected.value !== null && answering.has(selected.value.id),
  (isAnswering) => {
    if (!isAnswering) return
    const next = groups.value.find((g) => g.id === 'waiting')?.items.find((s) => !answering.has(s.id))
    if (next) select(next.id)
  },
)

const silentFor = computed(() => (selected.value ? silentMinutes(selected.value, now.value) : null))

function openPanel(session: ClaudeSession, event: MouseEvent) {
  select(session.id)
  if (target.value) panel.open(target.value, event)
}

defineExpose({ target, move, page, group })
</script>

<template>
  <ScreenFrame pane="wide">
    <template #strip><slot name="strip" /></template>

    <ErrorRow v-if="sessions.error.value" class="m-2" message="Sessions unavailable" @retry="sessions.reload" />
    <ListSkeleton v-if="showSkeleton" size="lg" />
    <EmptyState
      v-else-if="hasData && sessions.data.value?.length === 0"
      icon="bot"
      title="No sessions"
      hint="Sessions started in a terminal appear here"
    />
    <EmptyState v-else-if="hasData && shownCount === 0" icon="search-x" title="No match" />
    <ItemList
      v-else-if="hasData"
      :groups="groups"
      :selected-id="selectedId"
      row-size="lg"
      @update:selected-id="select"
      @activate="run('focus')"
      @context="openPanel"
    >
      <template #row="{ item, selected: isSelected }">
        <SessionRow :session="item" :selected="isSelected" :silent-for="silentMinutes(item, now)" />
      </template>
    </ItemList>

    <template #pane>
      <SessionDetail v-if="selected" :session="selected" :silent-for="silentFor" />
    </template>
  </ScreenFrame>
</template>
