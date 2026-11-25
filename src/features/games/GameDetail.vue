<script setup lang="ts">
// The detail pane of the selected game: cover, facts and the two launch buttons (spec 7.4).
import { computed } from 'vue'
import { runAction } from '@/actions/context'
import DetailPane from '@/components/rain/DetailPane.vue'
import KeyHint from '@/components/rain/KeyHint.vue'
import RelTime from '@/components/rain/RelTime.vue'
import RIcon from '@/components/rain/RIcon.vue'
import { Button } from '@/components/ui/button'
import { useImageFallback } from '@/composables/useImageFallback'
import { useScreenState } from '@/composables/useScreen'
import type { Game } from '@/data/types'
import { formatBytes, formatMinutes } from '@/lib/format'
import { keepFocus } from '@/lib/focus'
import { SOURCE_ICON, SOURCE_LABEL } from './gameSource'

const props = defineProps<{ game: Game }>()

// A cover that cannot be loaded shows the title, like a game that has no cover.
const cover = useImageFallback(() => props.game.coverUrl)

// The same actions Enter and Ctrl+Enter run, so the buttons and the keys cannot disagree.
const { primary, secondary } = useScreenState()

const state = computed(() => {
  const { game } = props
  if (game.running) return 'Running'
  if (game.source === 'custom') {
    if (!game.installed) return 'Program file missing'
    return game.exePath === undefined ? 'No program file' : 'Installed'
  }
  return game.installed ? 'Installed' : 'Not installed'
})
</script>

<template>
  <DetailPane :title="game.title" :status="game.running ? 'running' : undefined">
    <div class="flex flex-col gap-4">
      <div class="aspect-[2/3] w-[200px] overflow-hidden rounded-lg border border-border bg-surface-2">
        <div v-if="game.coverUrl && !cover.failed.value && game.coverIsIcon" class="flex size-full items-center justify-center">
          <img
            :src="game.coverUrl"
            alt=""
            decoding="async"
            draggable="false"
            class="size-3/5 object-contain"
            @error="cover.onError"
          >
        </div>
        <img
          v-else-if="game.coverUrl && !cover.failed.value"
          :src="game.coverUrl"
          alt=""
          decoding="async"
          draggable="false"
          class="size-full object-cover"
          @error="cover.onError"
        >
        <div v-else class="flex size-full items-center justify-center p-4">
          <span class="text-center text-sm font-medium text-foreground">{{ game.title }}</span>
        </div>
      </div>

      <dl class="grid grid-cols-[auto_minmax(0,1fr)] gap-x-4 gap-y-2 text-13">
        <dt class="text-muted-foreground">Source</dt>
        <dd class="flex min-w-0 items-center gap-2 text-foreground">
          <RIcon :icon="SOURCE_ICON[game.source]" :size="16" />
          <span class="truncate">{{ SOURCE_LABEL[game.source] }}</span>
        </dd>

        <dt class="text-muted-foreground">Status</dt>
        <dd class="text-foreground">{{ state }}</dd>

        <dt class="text-muted-foreground">Last played</dt>
        <dd class="text-foreground">
          <RelTime v-if="game.lastPlayedAt" :iso="game.lastPlayedAt" />
          <span v-else class="text-text-3">Never</span>
        </dd>

        <dt class="text-muted-foreground">Playtime</dt>
        <dd class="text-foreground">{{ formatMinutes(game.playtimeMinutes) }}</dd>

        <template v-if="game.sizeBytes !== undefined">
          <dt class="text-muted-foreground">Size</dt>
          <dd class="text-foreground">{{ formatBytes(game.sizeBytes) }}</dd>
        </template>
      </dl>
    </div>

    <template #actions>
      <Button
        v-if="primary"
        tabindex="-1"
        class="gap-2"
        @mousedown="keepFocus"
        @click="runAction(primary)"
      >
        {{ primary.label }}
        <KeyHint keys="Enter" />
      </Button>
      <button
        v-if="secondary"
        type="button"
        tabindex="-1"
        class="inline-flex h-9 items-center justify-center gap-2 rounded-md border border-border-strong px-4 text-sm font-medium text-foreground transition-colors hover:bg-hover"
        @mousedown="keepFocus"
        @click="runAction(secondary)"
      >
        {{ secondary.label }}
        <KeyHint keys="Ctrl+Enter" />
      </button>
    </template>
  </DetailPane>
</template>
