<script setup lang="ts">
// The detail pane of Life: the read-out of the selected cell, every habit on that
// day, the button for today's note and the agenda (passed in as a slot).
import { computed } from 'vue'
import DetailPane from '@/components/rain/DetailPane.vue'
import KeyHint from '@/components/rain/KeyHint.vue'
import RIcon from '@/components/rain/RIcon.vue'
import { Button } from '@/components/ui/button'
import { Separator } from '@/components/ui/separator'
import type { Habit, HabitDay, HabitValue, ISODate } from '@/data/types'
import { keepFocus } from '@/lib/focus'
import { parseISODate } from './dates'

const props = defineProps<{
  date: ISODate
  /** The selected cell in words: "Tue 3 Nov, Reading 30 min, streak 5". */
  readout: string
  habits: Habit[]
  day: HabitDay | undefined
  /** False while the vault cannot be found: there are no habits to list. */
  vaultFound: boolean
}>()

const emit = defineEmits<{ openToday: [] }>()

const title = computed(() =>
  new Intl.DateTimeFormat('en-GB', { weekday: 'long', day: 'numeric', month: 'long', year: 'numeric' }).format(
    parseISODate(props.date),
  ),
)

function valueText(habit: Habit, value: HabitValue | undefined): string {
  if (value === 'skip') return 'skipped'
  if (value === true) return 'Done'
  if (value === false) return 'Not done'
  if (typeof value === 'number') {
    const unit = habit.unit ? ` ${habit.unit}` : ''
    return habit.target ? `${value} / ${habit.target}${unit}` : `${value}${unit}`
  }
  return '–'
}
</script>

<template>
  <DetailPane :title="title">
    <template v-if="vaultFound">
      <p class="selectable mb-4 text-13 text-muted-foreground">{{ readout }}</p>
      <p v-if="!day?.noteExists" class="mb-4 text-13 text-muted-foreground">No note for this day</p>
      <ul v-else class="mb-4 flex flex-col">
        <li v-for="habit in habits" :key="habit.id" class="flex h-8 items-center gap-3 px-2 text-13">
          <RIcon :icon="habit.icon" :size="16" />
          <span class="min-w-0 flex-1 truncate text-foreground">{{ habit.name }}</span>
          <span class="shrink-0 text-muted-foreground">{{ valueText(habit, day.values[habit.id]) }}</span>
        </li>
      </ul>

      <Button variant="outline" tabindex="-1" class="gap-2" @mousedown="keepFocus" @click="emit('openToday')">
        Today's note
        <KeyHint keys="Ctrl+KeyT" />
      </Button>
      <Separator class="my-4" />
    </template>

    <slot />
  </DetailPane>
</template>
