<script setup lang="ts">
// The pane for the selected job: schedule, the Enabled switch, its last 20 runs and the two buttons.
import DetailPane from '@/components/rain/DetailPane.vue'
import KeyHint from '@/components/rain/KeyHint.vue'
import RelTime from '@/components/rain/RelTime.vue'
import { Button } from '@/components/ui/button'
import { Switch } from '@/components/ui/switch'
import type { ScheduledJob } from '@/data/types'
import { keepFocus } from '@/lib/focus'
import { formatDuration, formatClock, formatDayLabel } from '@/lib/time'
import { NO_PRESS } from '../buttonStyle'
import FactList from '../FactList.vue'
import { useRunAction } from '../useRunAction'
import { runFill } from './runColor'

/** The newest runs listed. */
const RUN_COUNT = 20

defineProps<{ job: ScheduledJob }>()

const run = useRunAction()
</script>

<template>
  <DetailPane :title="job.name" :status="job.running ? 'running' : (job.recentRuns[0]?.status ?? 'idle')">
    <div class="flex flex-col gap-4">
      <FactList>
        <dt>Cron</dt>
        <dd class="selectable font-mono text-xs">{{ job.cron }}</dd>
        <dt>Schedule</dt>
        <dd>{{ job.scheduleLabel }}</dd>
        <dt>Next run</dt>
        <dd>
          <RelTime v-if="job.enabled && job.nextRunAt" :iso="job.nextRunAt" />
          <span v-else class="text-muted-foreground">Not scheduled</span>
        </dd>
        <dt>Enabled</dt>
        <dd class="flex items-center gap-2">
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
          class="grid h-8 grid-cols-[8px_minmax(0,1fr)_auto_48px] items-center gap-3 text-13"
        >
          <span class="size-2 rounded-xs" :class="runFill(entry.status)" />
          <span class="truncate text-foreground">{{ formatDayLabel(entry.startedAt) }} {{ formatClock(entry.startedAt) }}</span>
          <span class="text-muted-foreground">{{ entry.durationMs === undefined ? 'running' : formatDuration(entry.durationMs) }}</span>
          <span class="text-right font-mono text-xs text-muted-foreground">
            {{ entry.exitCode === undefined ? '' : `exit ${entry.exitCode}` }}
          </span>
        </div>
      </section>
    </div>

    <template #actions>
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
