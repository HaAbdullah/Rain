<script setup lang="ts">
// The pane for the selected idle job: trigger, resource, progress, the Enabled switch and the two buttons.
import { CpuIcon, GpuIcon } from '@lucide/vue'
import DetailPane from '@/components/rain/DetailPane.vue'
import KeyHint from '@/components/rain/KeyHint.vue'
import Meter from '@/components/rain/Meter.vue'
import RelTime from '@/components/rain/RelTime.vue'
import { Button } from '@/components/ui/button'
import { Switch } from '@/components/ui/switch'
import type { IdleJob } from '@/data/types'
import { keepFocus } from '@/lib/focus'
import { NO_PRESS } from '../buttonStyle'
import FactList from '../FactList.vue'
import { useRunAction } from '../useRunAction'
import { idleStateText } from './idleState'

defineProps<{ job: IdleJob }>()

const run = useRunAction()
</script>

<template>
  <DetailPane :title="job.name" :status="job.state">
    <div class="flex flex-col gap-4">
      <FactList>
        <dt>State</dt>
        <dd>{{ idleStateText(job) }}</dd>
        <dt>Trigger</dt>
        <dd>After {{ job.idleAfterMinutes }} min idle</dd>
        <dt>Resource</dt>
        <dd class="flex items-center gap-2">
          <component :is="job.resource === 'gpu' ? GpuIcon : CpuIcon" :size="16" :stroke-width="1.75" class="text-muted-foreground" />
          {{ job.resource === 'gpu' ? 'GPU' : 'CPU' }}
        </dd>
        <dt>Last run</dt>
        <dd>
          <RelTime v-if="job.lastRunAt" :iso="job.lastRunAt" />
          <span v-else class="text-muted-foreground">Never</span>
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

      <div class="flex items-center gap-3">
        <Meter :value="job.progress" tone="primary" />
        <span class="w-9 shrink-0 text-right text-13 text-muted-foreground">{{ Math.round(job.progress * 100) }}%</span>
      </div>
    </div>

    <template #actions>
      <Button variant="outline" class="gap-2" :class="NO_PRESS" tabindex="-1" @mousedown="keepFocus" @click="run('log')">
        Log
        <KeyHint keys="Enter" />
      </Button>
      <Button variant="ghost" class="gap-2" :class="NO_PRESS" tabindex="-1" @mousedown="keepFocus" @click="run('run-now')">
        Run now
        <KeyHint keys="Ctrl+KeyR" />
      </Button>
    </template>
  </DetailPane>
</template>
