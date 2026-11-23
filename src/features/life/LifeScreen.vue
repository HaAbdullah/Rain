<script setup lang="ts">
// Life (spec 7.6): your habits, nothing else. A list grouped by Daily, Weekly and Monthly, each
// row a habit with today's checkbox and a small heat strip. The full progress of one habit is
// its own view, opened with Ctrl+Enter or a click on the row.
//
// Rain reads the vault, and writes exactly one thing: the checkbox you tick sets that one
// property in that day's daily note. There is no calendar and no agenda here.
import { TriangleAlertIcon } from '@lucide/vue'
import { computed, watch } from 'vue'
import { runPrimary } from '@/actions/context'
import type { ActionTarget } from '@/actions/types'
import EmptyState from '@/components/rain/EmptyState.vue'
import ErrorRow from '@/components/rain/ErrorRow.vue'
import ItemList from '@/components/rain/ItemList.vue'
import ScreenFrame from '@/components/rain/ScreenFrame.vue'
import { Button } from '@/components/ui/button'
import { Skeleton } from '@/components/ui/skeleton'
import { useActionPanel } from '@/composables/useActionPanel'
import { useNow } from '@/composables/useNow'
import { useResource } from '@/composables/useResource'
import { useScreen } from '@/composables/useScreen'
import { useSectionQuery } from '@/composables/useSearch'
import { useSelection } from '@/composables/useSelection'
import { api } from '@/data'
import { keepFocus } from '@/lib/focus'
import { toISODate } from '@/lib/time'
import { useNav } from '@/nav/useNav'
import { closeHabit, habitEditor, lifeView, openHabit } from './actions'
import HabitEditorDialog from './HabitEditorDialog.vue'
import HabitRow from './HabitRow.vue'
import HabitView from './HabitView.vue'
import { groupHabits, type HabitRow as HabitRowItem } from './rows'
import { useOpenHabit } from './useOpenHabit'

const nav = useNav()
const panel = useActionPanel()
const query = useSectionQuery('life')
const now = useNow()
const today = computed(() => toISODate(new Date(now.value)))

// ---- data ----

const vault = useResource('vault', () => api.getVaultStatus())
// The backend pushes the list after a tick and after a change in the notes or in habits.toml.
const habits = useResource('habits', () => api.listHabits(), { live: 'habits' })
// After midnight "today" is another day: the list is fetched again.
watch(today, () => void habits.reload())

const habitList = computed(() => habits.data.value ?? [])
const open = useOpenHabit(habitList, today)

const failed = computed(() => vault.error.value ?? habits.error.value)
const ready = computed(() => vault.data.value !== undefined && habits.data.value !== undefined)
const connected = computed(() => vault.data.value?.connected === true)
const writable = computed(() => vault.data.value?.writable === true)

function reloadAll() {
  void Promise.all([vault.reload(), habits.reload()])
}

// "Show in Life" can open a habit straight away; read once, like the pool of the Apps screen.
const wantedHabitId = nav.route.value.params?.id
if (wantedHabitId) openHabit(wantedHabitId)

// A habit that was deleted (or renamed by hand in habits.toml) while it was open: back to the list.
watch([() => habits.data.value, () => lifeView.habitId], () => {
  const list = habits.data.value
  if (list && lifeView.habitId !== '' && !list.some((summary) => summary.habit.id === lifeView.habitId)) {
    closeHabit()
  }
})

// ---- the list ----

const groups = computed(() => groupHabits(habitList.value, query.value))
const { selectedId, selected, move, page, group, select } = useSelection(groups, {
  resetOn: query,
  persistKey: 'life.selected',
})

function targetOf(row: HabitRowItem): ActionTarget {
  return { type: 'habit', id: row.id, item: row }
}

const noMatch = computed(
  () => habitList.value.length > 0 && groups.value.every((entry) => entry.items.length === 0),
)

function openPanel(row: HabitRowItem, event: MouseEvent) {
  select(row.id)
  panel.open(targetOf(row), event)
}

// ---- keys ----

const viewing = computed(() => open.habit.value !== undefined)

useScreen({
  target: computed<ActionTarget | null>(() => {
    if (viewing.value) {
      const day = open.dayItem.value
      return day ? { type: 'habitDay', id: day.date, item: day } : null
    }
    return selected.value ? targetOf(selected.value) : null
  }),
  // The list returns false for Left and Right, so the caret keeps them; the year grid takes all four.
  move: (dir) => (viewing.value ? open.move(dir) : move(dir)),
  page: (dir) => (viewing.value ? open.page(dir) : page(dir)),
  group: (dir) => {
    if (!viewing.value) group(dir)
  },
  // Esc and Backspace on an empty query leave the full view.
  back: leaveHabit,
  stepOut: leaveHabit,
})

function leaveHabit(): boolean {
  if (lifeView.habitId === '') return false
  closeHabit()
  return true
}

function openSettings() {
  nav.go('settings')
}
</script>

<template>
  <ScreenFrame pane="none">
    <!-- A failed refetch keeps the data the screen already has: the error shows above it, not instead of it. -->
    <div v-if="failed" class="px-4 pt-4" @mousedown="keepFocus">
      <ErrorRow message="Vault unavailable" :detail="failed.message" @retry="reloadAll" />
    </div>

    <div v-if="!ready" class="flex flex-col gap-2 p-4" aria-hidden="true">
      <template v-if="vault.showSkeleton.value || habits.showSkeleton.value">
        <Skeleton v-for="row in 5" :key="row" class="h-10" />
      </template>
    </div>

    <EmptyState
      v-else-if="!connected"
      icon="calendar-check"
      :title="vault.data.value?.path ? 'Vault not found' : 'Choose your vault'"
      :hint="
        vault.data.value?.path
          ? `Nothing at ${vault.data.value.path}. Check the path in Settings.`
          : 'Point Rain at your Obsidian vault to see your habits.'
      "
      action-label="Open settings"
      @action="openSettings"
    />

    <EmptyState
      v-else-if="habitList.length === 0"
      icon="calendar-check"
      title="No habits yet"
      hint="A habit is a checkbox property at the top of your daily note."
      action-label="New habit"
      action-keys="Ctrl+KeyN"
      @action="habitEditor.create()"
    />

    <HabitView
      v-else-if="open.habit.value"
      :summary="open.habit.value"
      :open="open"
      :today="today"
      :writable="writable"
      @back="closeHabit()"
      @edit="habitEditor.edit(open.habit.value.habit)"
      @tick-today="runPrimary({ type: 'habit', id: open.habit.value.habit.id, item: open.habit.value })"
    />

    <EmptyState v-else-if="noMatch" icon="search-x" title="No habit matches" />

    <template v-else>
      <!-- Only when there is something to say: a problem with the daily notes, or ticking is off. -->
      <div
        v-if="vault.data.value?.problem"
        class="flex h-10 shrink-0 items-center gap-3 px-4 text-13 text-muted-foreground"
      >
        <TriangleAlertIcon :size="16" :stroke-width="1.75" class="shrink-0 text-status-waiting" aria-hidden="true" />
        <span class="min-w-0 flex-1 truncate">{{ vault.data.value.problem }}</span>
        <Button variant="ghost" size="sm" tabindex="-1" @mousedown="keepFocus" @click="openSettings">
          Open settings
        </Button>
      </div>
      <p v-if="!writable" class="flex h-10 shrink-0 items-center px-4 text-13 text-muted-foreground">
        Ticking is switched off in this development build.
      </p>

      <ItemList
        :groups="groups"
        :selected-id="selectedId"
        click-runs
        @update:selected-id="select"
        @activate="openHabit($event.id)"
        @context="openPanel"
      >
        <template #row="{ item, selected: isSelected }">
          <HabitRow
            :item="item"
            :selected="isSelected"
            :writable="writable"
            @tick="runPrimary(targetOf(item))"
          />
        </template>
      </ItemList>
    </template>

    <HabitEditorDialog />
  </ScreenFrame>
</template>
