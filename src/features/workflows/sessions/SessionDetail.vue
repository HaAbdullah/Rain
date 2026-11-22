<script setup lang="ts">
// The pane for the selected session: facts, the request it waits on, recent events and the two buttons.
import {
  CircleAlertIcon,
  HandIcon,
  MessageCircleIcon,
  MessageSquareIcon,
  PencilIcon,
  WrenchIcon,
} from '@lucide/vue'
import type { Component } from 'vue'
import DetailPane from '@/components/rain/DetailPane.vue'
import KeyHint from '@/components/rain/KeyHint.vue'
import PathText from '@/components/rain/PathText.vue'
import RelTime from '@/components/rain/RelTime.vue'
import { Button } from '@/components/ui/button'
import type { ClaudeSession, SessionEvent } from '@/data/types'
import { keepFocus } from '@/lib/focus'
import { formatCount } from '@/lib/format'
import { formatClock } from '@/lib/time'
import { NO_PRESS } from '../buttonStyle'
import FactList from '../FactList.vue'
import { useRunAction } from '../useRunAction'
import RequestCard from './RequestCard.vue'

/** The newest events shown. */
const EVENT_COUNT = 8

defineProps<{
  session: ClaudeSession
  /** Minutes of silence when the session is stale, else null. */
  silentFor: number | null
}>()

const run = useRunAction()

const kindIcons: Record<SessionEvent['kind'], Component> = {
  prompt: MessageSquareIcon,
  tool: WrenchIcon,
  edit: PencilIcon,
  message: MessageCircleIcon,
  permission: HandIcon,
  error: CircleAlertIcon,
}

const kindTones: Partial<Record<SessionEvent['kind'], string>> = {
  permission: 'text-status-waiting',
  error: 'text-status-failed',
}
</script>

<template>
  <DetailPane :title="session.title" :status="silentFor === null ? session.status : 'stale'">
    <div class="flex flex-col gap-4">
      <PathText :path="session.cwd" />

      <FactList>
        <template v-if="session.branch">
          <dt>Branch</dt>
          <dd class="font-mono text-xs">{{ session.branch }}</dd>
        </template>
        <dt>Model</dt>
        <dd>{{ session.model }}</dd>
        <dt>Started</dt>
        <dd><RelTime :iso="session.startedAt" /></dd>
        <dt>Tokens</dt>
        <dd>{{ formatCount(session.tokens) }}</dd>
      </FactList>

      <p class="selectable line-clamp-3 text-13 text-foreground">{{ session.lastMessage }}</p>

      <RequestCard v-if="session.status === 'waiting' && session.request" :request="session.request" />

      <section class="flex flex-col gap-1" aria-label="Recent events">
        <h3 class="text-xs font-medium text-muted-foreground">Recent</h3>
        <div
          v-for="(event, index) in session.recent.slice(0, EVENT_COUNT)"
          :key="index"
          class="flex h-6 items-center gap-2"
        >
          <span class="shrink-0 font-mono text-xs text-text-3">{{ formatClock(event.at) }}</span>
          <component
            :is="kindIcons[event.kind]"
            :size="12"
            :stroke-width="1.75"
            class="shrink-0 text-muted-foreground"
            :class="kindTones[event.kind]"
            aria-hidden="true"
          />
          <span class="min-w-0 flex-1 truncate text-13 text-foreground">{{ event.text }}</span>
        </div>
      </section>
    </div>

    <template #actions>
      <Button variant="outline" class="gap-2" :class="NO_PRESS" tabindex="-1" @mousedown="keepFocus" @click="run('focus')">
        Focus terminal
        <KeyHint keys="Enter" />
      </Button>
      <Button variant="ghost" class="gap-2" :class="NO_PRESS" tabindex="-1" @mousedown="keepFocus" @click="run('transcript')">
        Transcript
        <KeyHint keys="Ctrl+KeyL" />
      </Button>
    </template>
  </DetailPane>
</template>
