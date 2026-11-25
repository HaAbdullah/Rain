<script setup lang="ts">
// Log viewer (spec 3.7): an optional list of runs on the left, the selected run's log on the right.
import { useDocumentVisibility } from '@vueuse/core'
import { onBeforeUnmount, ref, watch } from 'vue'
import { Dialog, DialogContent, DialogDescription, DialogTitle } from '@/components/ui/dialog'
import { ListboxContent, ListboxItem, ListboxRoot } from 'reka-ui'
import { ScrollArea } from '@/components/ui/scroll-area'
import { useLogDialog } from '@/composables/useLogDialog'
import type { ID, LogLine } from '@/data/types'
import { createFollower, type Follower } from '@/lib/follow'
import { formatClock, formatDayLabel, formatDuration, toISODate } from '@/lib/time'
import StatusDot from './StatusDot.vue'

const { isOpen, request, close } = useLogDialog()

const runId = ref<ID | undefined>()
const lines = ref<LogLine[]>([])
const loading = ref(false)
const failed = ref(false)

// Only the newest request may write its result: switching runs quickly must not show a stale log.
let latest = 0

/** True when the log was loaded; false when it could not be (the dialog then shows the failure). */
async function load(): Promise<boolean> {
  const current = request.value
  if (!current) return false
  const mine = ++latest
  let loaded = true
  loading.value = true
  failed.value = false
  try {
    const result = await current.load(runId.value)
    if (mine === latest) lines.value = result
  } catch {
    loaded = false
    if (mine === latest) {
      lines.value = []
      failed.value = true
    }
  } finally {
    if (mine === latest) loading.value = false
  }
  return loaded
}

// A run that is still going is followed: its log is loaded again every 3 seconds without
// clearing the lines, only while the dialog is open and the window is visible (UI spec rule 14).
const FOLLOW_EVERY_MS = 3000
const visibility = useDocumentVisibility()
let follower: Follower | null = null

function stopFollowing() {
  follower?.stop()
  follower = null
}

/** Loads the selected run, and keeps loading it while it is live. */
function show() {
  stopFollowing()
  const follow = request.value?.follow
  if (!follow) {
    void load()
    return
  }
  const shownRun = runId.value
  follower = createFollower({
    load: async () => {
      // A failed load ends the loop; the dialog shows the failure.
      if (!(await load())) throw new Error('the log could not be loaded')
    },
    isLive: () => follow(shownRun),
    everyMs: FOLLOW_EVERY_MS,
  })
  follower.start()
}

watch(isOpen, (open) => {
  if (!open) {
    stopFollowing()
    return
  }
  runId.value = request.value?.selectedRunId ?? request.value?.runs?.[0]?.id
  lines.value = []
  show()
})

// The loop stops while the window is hidden and starts again (with one load) when it shows.
watch(visibility, (state) => {
  if (!isOpen.value) return
  if (state === 'visible') show()
  else stopFollowing()
})

onBeforeUnmount(stopFollowing)

function selectRun(id: unknown) {
  runId.value = String(id)
  show()
}

function runLabel(iso: string): string {
  return `${formatDayLabel(toISODate(new Date(iso)))} ${formatClock(iso)}`
}

const barColor: Record<LogLine['level'], string> = {
  info: 'border-border-strong',
  warn: 'border-status-waiting',
  error: 'border-status-failed',
}
</script>

<template>
  <Dialog :open="isOpen" @update:open="(open) => !open && close()">
    <DialogContent
      :show-close-button="false"
      class="h-[560px] grid-rows-[minmax(0,1fr)] gap-0 overflow-hidden p-0 sm:max-w-[880px]"
    >
      <DialogDescription class="sr-only">Output of the selected run.</DialogDescription>
      <div v-if="request" class="flex h-full min-h-0">
        <ListboxRoot
          v-if="request.runs"
          :model-value="runId"
          class="flex w-60 shrink-0 flex-col gap-0.5 overflow-y-auto border-r border-border bg-surface-1 p-2 outline-none"
          @update:model-value="selectRun"
        >
          <ListboxContent class="flex flex-col gap-0.5">
            <ListboxItem
              v-for="run in request.runs"
              :key="run.id"
              :value="run.id"
              class="flex h-9 items-center gap-2 rounded-md px-2 text-13 outline-none transition-colors data-highlighted:bg-hover data-[state=checked]:bg-selected"
            >
              <StatusDot :status="run.status" :size="12" />
              <span class="min-w-0 flex-1 truncate">{{ runLabel(run.startedAt) }}</span>
              <span v-if="run.durationMs !== undefined" class="shrink-0 text-xs text-muted-foreground">
                {{ formatDuration(run.durationMs) }}
              </span>
            </ListboxItem>
          </ListboxContent>
        </ListboxRoot>

        <div class="flex min-w-0 flex-1 flex-col">
          <div class="flex h-10 shrink-0 items-center border-b border-border px-4">
            <DialogTitle class="truncate text-sm font-semibold">{{ request.title }}</DialogTitle>
          </div>
          <ScrollArea class="min-h-0 flex-1 bg-surface-0">
            <p v-if="failed" class="p-4 text-13 text-status-failed">Could not load the log.</p>
            <p v-else-if="loading && lines.length === 0" class="p-4 text-13 text-muted-foreground">Loading</p>
            <p v-else-if="lines.length === 0" class="p-4 text-13 text-muted-foreground">No output.</p>
            <div v-else class="selectable py-2 font-mono text-xs leading-5">
              <div
                v-for="(line, index) in lines"
                :key="index"
                class="flex gap-3 border-l-2 px-3 py-px"
                :class="barColor[line.level]"
              >
                <span class="shrink-0 text-text-3">{{ formatClock(line.ts) }}</span>
                <span class="min-w-0 whitespace-pre-wrap break-words">{{ line.text }}</span>
              </div>
            </div>
          </ScrollArea>
        </div>
      </div>
    </DialogContent>
  </Dialog>
</template>
