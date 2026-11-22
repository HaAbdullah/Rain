<script setup lang="ts">
// The permission a waiting session asks for: the tool, the exact command or diff, and the three answers.
import { computed } from 'vue'
import KeyHint from '@/components/rain/KeyHint.vue'
import RelTime from '@/components/rain/RelTime.vue'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import type { PermissionRequest } from '@/data/types'
import { keepFocus } from '@/lib/focus'
import { NO_PRESS } from '../buttonStyle'
import { useRunAction } from '../useRunAction'

const props = defineProps<{ request: PermissionRequest }>()

const run = useRunAction()

/** The detail line by line, with added and removed lines tinted when it is a diff. */
const lines = computed(() => {
  const isDiff = /^(---|\+\+\+|@@)/m.test(props.request.detail)
  return props.request.detail.split('\n').map((text) => {
    if (!isDiff || text.startsWith('---') || text.startsWith('+++')) return { text, tone: '' }
    if (text.startsWith('+')) return { text, tone: 'text-status-success' }
    if (text.startsWith('-')) return { text, tone: 'text-status-failed' }
    return { text, tone: '' }
  })
})
</script>

<template>
  <section class="flex flex-col gap-3 rounded-lg border border-border-strong bg-surface-2 p-3" aria-label="Permission request">
    <div class="flex items-center gap-2">
      <Badge class="bg-status-waiting/15 text-status-waiting">{{ request.tool }}</Badge>
      <span class="text-13 text-muted-foreground">requested <RelTime :iso="request.requestedAt" /></span>
    </div>

    <pre
      class="selectable max-h-[220px] overflow-auto rounded-md bg-surface-0 p-3 font-mono text-xs whitespace-pre text-foreground"
    ><span v-for="(line, index) in lines" :key="index" class="block min-h-4" :class="line.tone">{{ line.text }}</span></pre>

    <div class="flex flex-wrap items-center gap-2">
      <Button size="sm" class="gap-2" tabindex="-1" @mousedown="keepFocus" @click="run('allow-once')">
        Allow once
        <KeyHint keys="Ctrl+KeyY" />
      </Button>
      <Button variant="outline" size="sm" :class="NO_PRESS" tabindex="-1" @mousedown="keepFocus" @click="run('allow-always')">
        Always allow
      </Button>
      <Button variant="ghost" size="sm" class="gap-2" :class="NO_PRESS" tabindex="-1" @mousedown="keepFocus" @click="run('deny')">
        Deny
        <KeyHint keys="Ctrl+KeyD" />
      </Button>
    </div>
  </section>
</template>
