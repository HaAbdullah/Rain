<script setup lang="ts">
// One habit in the Life list: icon, name, progress (weekly and monthly), the heat strip and
// today's checkbox. The checkbox ticks; a click anywhere else on the row opens the habit.
import RIcon from '@/components/rain/RIcon.vue'
import { Checkbox } from '@/components/ui/checkbox'
import type { HabitSummary } from '@/data/types'
import { keepFocus } from '@/lib/focus'
import HabitStrip from './HabitStrip.vue'

defineProps<{
  item: HabitSummary
  selected: boolean
  /** False when Rain may not tick (a development build with a real vault): the box is disabled. */
  writable: boolean
}>()

const emit = defineEmits<{ tick: [] }>()
</script>

<template>
  <div
    class="flex h-(--row-h) items-center gap-3 rounded-lg px-2 transition-colors"
    :class="selected ? 'bg-selected' : 'hover:bg-hover'"
  >
    <RIcon :icon="item.habit.icon" :size="20" :label="item.habit.name" />
    <span class="min-w-0 flex-1 truncate text-sm text-foreground">{{ item.habit.name }}</span>
    <span
      v-if="item.habit.cadence !== 'daily'"
      class="w-12 shrink-0 text-right text-13"
      :class="item.periodCount >= item.periodTarget ? 'text-foreground' : 'text-muted-foreground'"
    >
      {{ item.periodCount }} / {{ item.periodTarget }}
    </span>
    <HabitStrip :cells="item.cells" :cadence="item.habit.cadence" />
    <!-- @click.stop: the click is a tick, not "open the row". Mousedown keeps focus in the search field. -->
    <Checkbox
      :model-value="item.todayDone"
      :disabled="!writable"
      tabindex="-1"
      :aria-label="`${item.habit.name}, today`"
      @click.stop
      @mousedown="keepFocus"
      @update:model-value="emit('tick')"
    />
  </div>
</template>
