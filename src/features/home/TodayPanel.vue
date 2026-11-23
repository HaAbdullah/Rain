<script setup lang="ts">
// The read-only side of Home: today's date, one tile per habit and the "Today's note" button.
// Ticking happens on the Life screen, never here. It is not part of the keyboard cursor.
import { NotebookPenIcon } from '@lucide/vue'
import { computed } from 'vue'
import { commands } from '@/actions/commands'
import { runPrimary } from '@/actions/context'
import ErrorRow from '@/components/rain/ErrorRow.vue'
import KeyHint from '@/components/rain/KeyHint.vue'
import RIcon from '@/components/rain/RIcon.vue'
import { Button } from '@/components/ui/button'
import { Separator } from '@/components/ui/separator'
import { Skeleton } from '@/components/ui/skeleton'
import { Tooltip, TooltipContent, TooltipTrigger } from '@/components/ui/tooltip'
import { useNow } from '@/composables/useNow'
import { useResource } from '@/composables/useResource'
import { api } from '@/data'
import { keepFocus } from '@/lib/focus'
import { formatDayLabel, toISODate } from '@/lib/time'
import { useNav } from '@/nav/useNav'

const MAX_HABITS = 7

const nav = useNav()
// Follows the clock, so the date changes at midnight without a remount.
const now = useNow()
const today = computed(() => toISODate(new Date(now.value)))

// The same resource and topic as the Life screen: one fetch feeds both.
const habits = useResource('habits', () => api.listHabits(), { live: 'habits' })
const shown = computed(() => (habits.data.value ?? []).slice(0, MAX_HABITS))
const doneCount = computed(() => shown.value.filter((item) => item.todayDone).length)

const todayNote = commands.find((command) => command.id === 'today-note')

function openTodayNote() {
  if (todayNote) runPrimary({ type: 'command', id: todayNote.id, item: todayNote })
}
</script>

<template>
  <div class="flex h-full flex-col gap-4 p-4">
    <div class="flex items-baseline justify-between">
      <h2 class="text-base font-semibold text-foreground">Today</h2>
      <span class="text-xs text-muted-foreground">{{ formatDayLabel(today) }}</span>
    </div>

    <Separator />

    <div v-if="habits.error.value" class="-mx-2">
      <ErrorRow message="Habits unavailable" :detail="habits.error.value.message" @retry="habits.reload()" />
    </div>
    <div v-else-if="habits.showSkeleton.value" class="flex gap-2" aria-hidden="true">
      <Skeleton v-for="n in MAX_HABITS" :key="n" class="size-7 rounded-md" />
    </div>
    <p v-else-if="habits.data.value && shown.length === 0" class="text-13 text-muted-foreground">
      <button
        type="button"
        tabindex="-1"
        class="underline-offset-2 transition-colors hover:text-foreground hover:underline"
        @mousedown="keepFocus"
        @click="nav.go('life')"
      >
        No habits yet. Add one in Life
      </button>
    </p>
    <template v-else>
      <div class="flex flex-wrap gap-2">
        <Tooltip v-for="item in shown" :key="item.habit.id">
          <TooltipTrigger as-child>
            <button
              type="button"
              tabindex="-1"
              class="flex size-7 items-center justify-center rounded-md transition-colors"
              :class="item.todayDone ? 'bg-heat-3 text-foreground' : 'bg-surface-2 text-muted-foreground'"
              :aria-label="`${item.habit.name}: ${item.todayDone ? 'done' : 'not yet'}`"
              @mousedown="keepFocus"
              @click="nav.go('life')"
            >
              <RIcon :icon="item.habit.icon" :size="16" />
            </button>
          </TooltipTrigger>
          <TooltipContent>{{ item.habit.name }}: {{ item.todayDone ? 'done' : 'not yet' }}</TooltipContent>
        </Tooltip>
      </div>
      <p v-if="shown.length > 0" class="text-xs text-muted-foreground">
        {{ doneCount }} of {{ shown.length }} done today
      </p>
    </template>

    <Button
      v-if="todayNote"
      variant="outline"
      class="mt-auto gap-2"
      tabindex="-1"
      @mousedown="keepFocus"
      @click="openTodayNote"
    >
      <NotebookPenIcon :size="16" :stroke-width="1.75" aria-hidden="true" />
      Today's note
      <KeyHint :keys="todayNote.keys ?? 'Ctrl+KeyT'" />
    </Button>
  </div>
</template>
