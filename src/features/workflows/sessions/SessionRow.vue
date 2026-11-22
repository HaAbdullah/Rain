<script setup lang="ts">
// One session in the list. A waiting session shows its request on line two and an amber bar on the left.
import { computed } from 'vue'
import RelTime from '@/components/rain/RelTime.vue'
import StatusDot from '@/components/rain/StatusDot.vue'
import type { ClaudeSession } from '@/data/types'
import { folderName } from './sessionInfo'

const props = defineProps<{
  session: ClaudeSession
  selected: boolean
  /** Minutes of silence when the session is stale, else null. */
  silentFor: number | null
}>()

const request = computed(() => (props.session.status === 'waiting' ? props.session.request : undefined))
const quietLabel = computed(() => (props.silentFor === null ? undefined : `No activity for ${props.silentFor} m`))
</script>

<template>
  <div
    class="relative flex h-(--row-lg-h) items-center gap-3 rounded-lg px-2 transition-colors"
    :class="selected ? 'bg-selected' : 'hover:bg-hover'"
  >
    <span
      v-if="session.status === 'waiting'"
      class="absolute inset-y-2 left-0 w-0.5 rounded-full bg-status-waiting"
      aria-hidden="true"
    />
    <StatusDot :status="silentFor === null ? session.status : 'stale'" :label="quietLabel" />

    <div class="flex min-w-0 flex-1 flex-col justify-center">
      <span class="truncate text-sm text-foreground">{{ session.title }}</span>
      <span v-if="request" class="truncate text-13 text-foreground">{{ request.tool }}: {{ request.summary }}</span>
      <span v-else class="flex min-w-0 gap-2 font-mono text-xs text-muted-foreground">
        <span class="min-w-0 truncate">{{ folderName(session.cwd) }}</span>
        <span v-if="session.branch" class="max-w-[55%] shrink-0 truncate">{{ session.branch }}</span>
      </span>
    </div>

    <RelTime :iso="session.lastActivityAt" short class="text-13 text-muted-foreground" />
  </div>
</template>
