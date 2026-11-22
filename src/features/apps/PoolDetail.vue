<script setup lang="ts">
// The detail pane for a selected pool: last launch, its members, and while it launches
// (or after a partial failure) the launch checklist with "Retry failed".
import { computed, ref, watch } from 'vue'
import DetailPane from '@/components/rain/DetailPane.vue'
import KeyHint from '@/components/rain/KeyHint.vue'
import LaunchChecklist from '@/components/rain/LaunchChecklist.vue'
import RelTime from '@/components/rain/RelTime.vue'
import RIcon from '@/components/rain/RIcon.vue'
import { Button } from '@/components/ui/button'
import { useLaunch } from '@/composables/useLaunch'
import type { AppEntry, IconRef, Pool, PoolMember } from '@/data/types'
import { keepFocus } from '@/lib/focus'

const props = defineProps<{ pool: Pool; apps: AppEntry[] }>()
const emit = defineEmits<{ launch: []; edit: [] }>()

const { runs, launch } = useLaunch()

const UNKNOWN_APP_ICON: IconRef = { kind: 'lucide', value: 'app-window' }

const appById = computed(() => new Map(props.apps.map((app) => [app.id, app])))

/** The id of a finished launch that the user has moved past; it is not shown again. */
const dismissedRunId = ref<string | null>(null)

// A selection change clears an old result: only a launch that is still running stays.
watch(
  () => props.pool.id,
  (id) => {
    const run = runs.value[id]
    dismissedRunId.value = run && run.status !== 'running' ? run.id : null
  },
  { immediate: true },
)

/** The launch to show instead of the members: running, or failed in part. */
const run = computed(() => {
  const latest = runs.value[props.pool.id]
  const visible = latest && latest.status !== 'success' && latest.id !== dismissedRunId.value
  // A launch that could not even start has no steps; the notice already said so.
  return visible && latest.steps.length > 0 ? latest : undefined
})

const heading = computed(() => {
  if (!run.value) return 'Members'
  return run.value.status === 'running' ? 'Launching' : 'Last launch'
})

const failedCount = computed(() => run.value?.steps.filter((step) => step.status === 'failed').length ?? 0)
const retrying = ref(false)

/** Starts again only the members whose step failed (steps and members have the same order). */
async function retryFailed() {
  const current = run.value
  if (!current) return
  retrying.value = true
  try {
    const failed = props.pool.members.filter((_, i) => current.steps[i]?.status === 'failed')
    let allStarted = true
    for (const member of failed) {
      const result = await launch({ type: 'app', id: member.appId }, appById.value.get(member.appId)?.name)
      if (result.status !== 'success') allStarted = false
    }
    // Everything is running now, so the pool's failed run is forgotten (pane and row dot).
    if (allStarted) delete runs.value[props.pool.id]
  } finally {
    retrying.value = false
  }
}

function delayLabel(member: PoolMember): string | undefined {
  return member.delayMs ? `+${member.delayMs / 1000} s` : undefined
}
</script>

<template>
  <DetailPane :title="pool.name" :icon="pool.icon" :status="run?.status">
    <div class="flex items-center justify-between text-13">
      <span class="text-muted-foreground">Last launched</span>
      <RelTime v-if="pool.lastLaunchedAt" :iso="pool.lastLaunchedAt" class="text-foreground" />
      <span v-else class="text-muted-foreground">Never</span>
    </div>

    <h3 class="mt-4 mb-1 text-xs font-medium text-muted-foreground">{{ heading }}</h3>

    <template v-if="run">
      <LaunchChecklist :steps="run.steps" />
      <div v-if="failedCount > 0" class="mt-3 flex items-center gap-3">
        <p class="min-w-0 flex-1 text-13 text-status-failed">
          {{ failedCount }} of {{ run.steps.length }} did not start
        </p>
        <Button
          variant="outline"
          size="sm"
          tabindex="-1"
          class="active:translate-y-0 transition-colors"
          :disabled="retrying"
          @mousedown="keepFocus"
          @click="retryFailed"
        >
          Retry failed
        </Button>
      </div>
    </template>

    <ul v-else>
      <li v-for="member in pool.members" :key="member.appId" class="flex h-10 items-center gap-3">
        <RIcon :icon="appById.get(member.appId)?.icon ?? UNKNOWN_APP_ICON" :size="20" />
        <span
          class="max-w-[55%] shrink-0 truncate text-sm"
          :class="appById.has(member.appId) ? 'text-foreground' : 'text-muted-foreground'"
        >
          {{ appById.get(member.appId)?.name ?? 'Unknown app' }}
        </span>
        <span
          class="min-w-0 flex-1 truncate font-mono text-xs text-muted-foreground"
          :title="member.args"
        >
          {{ member.args }}
        </span>
        <span v-if="delayLabel(member)" class="shrink-0 text-13 text-muted-foreground">
          {{ delayLabel(member) }}
        </span>
      </li>
    </ul>

    <template #actions>
      <Button
        tabindex="-1"
        class="gap-2"
        :disabled="run?.status === 'running'"
        @mousedown="keepFocus"
        @click="emit('launch')"
      >
        Launch all
        <KeyHint keys="Enter" />
      </Button>
      <Button variant="ghost" tabindex="-1" class="gap-2 active:translate-y-0 transition-colors" @mousedown="keepFocus" @click="emit('edit')">
        Edit
        <KeyHint keys="Ctrl+Enter" />
      </Button>
    </template>
  </DetailPane>
</template>
