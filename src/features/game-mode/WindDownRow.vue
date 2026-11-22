<script setup lang="ts">
// One line of the game-mode checklist: what will be stopped, how many, whether it loses work,
// and (while waiting or winding down) how far it has got.
import { computed } from 'vue'
import RIcon from '@/components/rain/RIcon.vue'
import StatusDot from '@/components/rain/StatusDot.vue'
import { Badge } from '@/components/ui/badge'
import type { GameModeState, WindDownItem } from '@/data/types'
import type { StatusKind } from '@/lib/status'

const props = defineProps<{
  item: WindDownItem
  phase: GameModeState['phase']
  /** True while review waits for interrupting work to finish. */
  waiting: boolean
}>()

const WIND_DOWN_STATUS: Record<WindDownItem['status'], StatusKind> = {
  pending: 'idle',
  stopping: 'running',
  stopped: 'success',
}

const progress = computed<StatusKind | null>(() => {
  if (props.phase === 'windingDown') return WIND_DOWN_STATUS[props.item.status]
  return props.waiting && props.item.effect === 'interrupts' ? 'running' : null
})
</script>

<template>
  <div class="flex h-(--row-h) items-center gap-3 px-3">
    <RIcon :icon="item.icon" :size="20" />
    <span class="min-w-0 flex-1 truncate text-sm text-foreground">{{ item.label }}</span>
    <span class="text-13 text-muted-foreground tabular-nums">{{ item.count }}</span>
    <Badge
      variant="secondary"
      class="w-20 text-muted-foreground"
      :class="{ 'bg-[color-mix(in_srgb,var(--status-waiting)_15%,transparent)] text-status-waiting': item.effect === 'interrupts' }"
    >
      {{ item.effect === 'interrupts' ? 'interrupts' : 'safe' }}
    </Badge>
    <span class="flex size-4 shrink-0 items-center justify-center">
      <StatusDot v-if="progress" :status="progress" />
    </span>
  </div>
</template>
