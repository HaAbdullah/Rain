<script setup lang="ts">
// The detail pane for a selected app: path, running state, the pools it belongs to.
import { computed } from 'vue'
import DetailPane from '@/components/rain/DetailPane.vue'
import KeyHint from '@/components/rain/KeyHint.vue'
import PathText from '@/components/rain/PathText.vue'
import RIcon from '@/components/rain/RIcon.vue'
import StatusDot from '@/components/rain/StatusDot.vue'
import { Button } from '@/components/ui/button'
import { useLaunch } from '@/composables/useLaunch'
import type { AppEntry, Pool, Status } from '@/data/types'
import { keepFocus } from '@/lib/focus'

const props = defineProps<{ app: AppEntry; pools: Pool[] }>()
const emit = defineEmits<{ launch: []; reveal: [] }>()

const { runs } = useLaunch()

const inPools = computed(() =>
  props.pools.filter((pool) => pool.members.some((member) => member.appId === props.app.id)),
)

/** What the state line says: a launch in progress or failed wins over the app's own state. */
const state = computed<{ status: Status; text: string }>(() => {
  const run = runs.value[props.app.id]
  if (run?.status === 'running') return { status: 'running', text: 'Starting' }
  if (run?.status === 'failed') {
    const error = run.steps.find((step) => step.status === 'failed')?.error
    return { status: 'failed', text: error ?? 'Failed to start' }
  }
  return props.app.running
    ? { status: 'success', text: 'Running' }
    : { status: 'idle', text: 'Not running' }
})
</script>

<template>
  <DetailPane :title="app.name">
    <RIcon :icon="app.icon" :size="32" class="mb-3" />
    <PathText :path="app.exePath" />

    <div class="mt-4 flex items-center gap-2 text-13">
      <StatusDot :status="state.status" :size="12" />
      <span class="min-w-0 truncate" :class="state.status === 'failed' ? 'text-status-failed' : 'text-foreground'">
        {{ state.text }}
      </span>
    </div>

    <h3 class="mt-4 mb-1 text-xs font-medium text-muted-foreground">In pools</h3>
    <ul v-if="inPools.length > 0">
      <li v-for="pool in inPools" :key="pool.id" class="flex h-8 items-center gap-3">
        <RIcon :icon="pool.icon" :size="16" />
        <span class="min-w-0 truncate text-sm text-foreground">{{ pool.name }}</span>
      </li>
    </ul>
    <p v-else class="text-13 text-muted-foreground">Not in any pool</p>

    <template #actions>
      <Button
        tabindex="-1"
        class="gap-2"
        :disabled="state.status === 'running'"
        @mousedown="keepFocus"
        @click="emit('launch')"
      >
        Launch
        <KeyHint keys="Enter" />
      </Button>
      <Button variant="ghost" tabindex="-1" class="gap-2 active:translate-y-0 transition-colors" @mousedown="keepFocus" @click="emit('reveal')">
        Open file location
        <KeyHint keys="Ctrl+Enter" />
      </Button>
    </template>
  </DetailPane>
</template>
