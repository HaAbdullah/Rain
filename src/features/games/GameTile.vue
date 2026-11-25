<script setup lang="ts">
// One game in the grid: cover with source badge, and the title under it (spec 7.4).
import { DownloadIcon } from '@lucide/vue'
import Tile from '@/components/rain/Tile.vue'
import RIcon from '@/components/rain/RIcon.vue'
import { useImageFallback } from '@/composables/useImageFallback'
import type { Game } from '@/data/types'
import { SOURCE_ICON } from './gameSource'

const props = defineProps<{
  game: Game
  selected: boolean
}>()

// A cover that cannot be loaded shows the title tile, like a game that has no cover.
const cover = useImageFallback(() => props.game.coverUrl)
</script>

<template>
  <div class="min-w-0">
    <div class="aspect-[2/3]">
      <Tile :selected="selected" :ring="game.running ? 'success' : 'none'">
        <div class="relative h-full overflow-hidden rounded-lg">
          <!-- The icon of a program file is a small square picture: centred on a plain tile, not stretched over it. -->
          <div
            v-if="game.coverUrl && !cover.failed.value && game.coverIsIcon"
            class="flex size-full items-center justify-center bg-surface-2"
          >
            <img
              :src="game.coverUrl"
              alt=""
              loading="lazy"
              decoding="async"
              draggable="false"
              class="size-3/5 object-contain transition-opacity"
              :class="{ 'opacity-45': !game.installed }"
              @error="cover.onError"
            >
          </div>
          <img
            v-else-if="game.coverUrl && !cover.failed.value"
            :src="game.coverUrl"
            alt=""
            loading="lazy"
            decoding="async"
            draggable="false"
            class="size-full object-cover transition-opacity"
            :class="{ 'opacity-45': !game.installed }"
            @error="cover.onError"
          >
          <div v-else class="flex size-full items-center justify-center bg-surface-2 p-3">
            <span class="line-clamp-4 text-center text-sm font-medium text-foreground">{{ game.title }}</span>
          </div>

          <!-- Only a game that could be downloaded shows the glyph; a local game whose program is gone is just dimmed. -->
          <div v-if="!game.installed && game.source !== 'custom'" class="absolute inset-0 flex items-center justify-center">
            <DownloadIcon :size="20" :stroke-width="1.75" class="text-foreground" aria-hidden="true" />
          </div>

          <span class="absolute bottom-1.5 left-1.5 flex size-5 items-center justify-center rounded-full bg-surface-1 text-muted-foreground">
            <RIcon :icon="SOURCE_ICON[game.source]" :size="12" />
          </span>
        </div>
      </Tile>
    </div>
    <p class="mt-1.5 truncate text-xs text-foreground">{{ game.title }}</p>
  </div>
</template>
