<script setup lang="ts">
// The pane for the selected job: schedule, the Enabled switch, its last 20 runs and the two buttons.
import { computed } from 'vue'
import DetailPane from '@/components/rain/DetailPane.vue'
import KeyHint from '@/components/rain/KeyHint.vue'
import RelTime from '@/components/rain/RelTime.vue'
import { Button } from '@/components/ui/button'
import { Switch } from '@/components/ui/switch'
import type { JobRun, ScheduledJob } from '@/data/types'
import { keepFocus } from '@/lib/focus'
import { formatDuration, formatClock, formatDayLabel } from '@/lib/time'
import { NO_PRESS } from '../buttonStyle'
import FactList from '../FactList.vue'
import { useRunAction } from '../useRunAction'
import { runFill } from './runColor'

/** The newest runs listed. */
const RUN_COUNT = 20

const props = defineProps<{ job: ScheduledJob }>()

const run = useRunAction()

/** A run of the Claude app that has no length shows nothing: the app does not say how long it took. */
function durationText(entry: JobRun): string {
  if (entry.durationMs !== undefined) return formatDuration(entry.durationMs)
  return entry.status === 'running' ? 'running' : ''
}

/** A job of the Claude app: Rain only shows it. No switch, no Run now. */
const fromApp = computed(() => props.job.source === 'claude-app')
</script>

<template>
  <DetailPane :title="job.name" :status="job.running ? 'running' : (job.recentRuns[0]?.status ?? 'idle')">
    <div class="flex flex-col gap-4">
      <p v-if="fromApp" class="text-13 text-muted-foreground">This job runs in the Claude app. Rain only shows it.</p>

      <FactList>
        <template v-if="job.cron">
          <dt>Cron</dt>
          <dd class="selectable font-mono text-xs">{{ job.cron }}</dd>
          <dt>Schedule</dt>
          <dd>{{ job.scheduleLabel }}</dd>
        </template>
        <dt>Next run</dt>
        <dd>
          <RelTime v-if="job.enabled && job.nextRunAt" :iso="job.nextRunAt" />
          <span v-else class="text-muted-foreground">Not scheduled</span>
        </dd>
        <dt>Enabled</dt>
        <dd v-if="fromApp">{{ job.enabled ? 'On' : 'Off' }}</dd>
        <dd v-else class="flex items-center gap-2">
          <Switch
            :model-value="job.enabled"
            tabindex="-1"
            aria-label="Enabled"
            @mousedown="keepFocus"
            @update:model-value="run('toggle')"
          />
          <KeyHint keys="Ctrl+Enter" />
        </dd>
      </FactList>

      <section class="flex flex-col" aria-label="Recent runs">
        <h3 class="mb-1 text-xs font-medium text-muted-foreground">Last {{ Math.min(job.recentRuns.length, RUN_COUNT) }} runs</h3>
        <p v-if="job.recentRuns.length === 0" class="text-13 text-muted-foreground">No runs yet</p>
        <div
          v-for="entry in job.recentRuns.slice(0, RUN_COUNT)"
          :key="entry.id"
          class="grid min-h-8 grid-cols-[8px_minmax(0,1fr)_auto_48px] items-center gap-x-3 text-13"
        >
          <span class="size-2 rounded-xs" :class="runFill(entry.status)" />
          <span class="truncate text-foreground">{{ formatDayLabel(entry.startedAt) }} {{ formatClock(entry.startedAt) }}</span>
          <span class="text-muted-foreground">{{ durationText(entry) }}</span>
          <span class="text-right font-mono text-xs text-muted-foreground">
            {{ entry.exitCode === undefined ? '' : `exit ${entry.exitCode}` }}
          </span>
          <p
            v-if="entry.outcome"
            class="col-span-full -mt-1 mb-1 truncate pl-5 text-xs"
            :class="entry.status === 'failed' ? 'text-status-failed' : 'text-muted-foreground'"
            :title="entry.outcome"
          >
            {{ entry.outcome }}
          </p>
        </div>
      </section>
    </div>

    <template v-if="!fromApp" #actions>
      <Button variant="outline" class="gap-2" :class="NO_PRESS" tabindex="-1" @mousedown="keepFocus" @click="run('log')">
        Runs and log
        <KeyHint keys="Enter" />
      </Button>
      <Button variant="ghost" class="gap-2" :class="NO_PRESS" tabindex="-1" @mousedown="keepFocus" @click="run('run-now')">
        Run now
        <KeyHint keys="Ctrl+KeyR" />
      </Button>
    </template>
  </DetailPane>
</template>
