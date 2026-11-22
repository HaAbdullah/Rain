<script setup lang="ts">
// A project in the grid: the large card (Most active) or the small one (All projects).
import { GitBranchIcon } from '@lucide/vue'
import { computed } from 'vue'
import IconStack from '@/components/rain/IconStack.vue'
import RelTime from '@/components/rain/RelTime.vue'
import StatusDot from '@/components/rain/StatusDot.vue'
import Tile from '@/components/rain/Tile.vue'
import type { Project } from '@/data/types'
import type { StatusKind } from '@/lib/status'
import CommitStrip from './CommitStrip.vue'
import ProjectCardName from './ProjectCardName.vue'

const props = defineProps<{
  project: Project
  selected: boolean
  size: 'lg' | 'sm'
  /** Status of the Claude session that lives in this project, if any (large card only). */
  sessionStatus?: StatusKind
}>()

const environmentIcons = computed(() => props.project.environment.map((step) => step.icon))
</script>

<template>
  <!-- The ratio and height sit here so the Tile can fill them without clipping its selection ring. -->
  <div class="min-h-0" :class="size === 'lg' ? 'aspect-video' : 'h-22'">
    <Tile :selected="selected" :dimmed="project.missing">
      <div v-if="size === 'lg'" class="flex h-full min-h-0 flex-col gap-1 overflow-hidden p-3">
        <div class="flex items-center gap-2">
          <ProjectCardName :project="project" :icon-size="32" name-class="text-base font-semibold" />
          <span class="flex-1" />
          <StatusDot v-if="sessionStatus" :status="sessionStatus" label="Claude session" />
          <RelTime :iso="project.lastOpenedAt" short class="text-xs text-muted-foreground" />
        </div>
        <div class="flex items-center gap-2 text-muted-foreground">
          <GitBranchIcon :size="12" :stroke-width="1.75" class="shrink-0" aria-hidden="true" />
          <span class="min-w-0 flex-1 truncate font-mono text-xs">{{ project.branch }}</span>
          <IconStack :icons="environmentIcons" :size="14" />
        </div>
        <CommitStrip class="mt-auto shrink-0" :counts="project.commits14d" />
      </div>

      <div v-else class="flex h-full min-h-0 flex-col justify-center gap-2 overflow-hidden p-3">
        <div class="flex items-center gap-2">
          <ProjectCardName :project="project" :icon-size="24" name-class="text-sm font-medium" />
        </div>
        <div class="flex items-center gap-2 text-muted-foreground">
          <GitBranchIcon :size="12" :stroke-width="1.75" class="shrink-0" aria-hidden="true" />
          <span class="min-w-0 flex-1 truncate font-mono text-xs">{{ project.branch }}</span>
          <RelTime :iso="project.lastOpenedAt" short class="text-xs" />
        </div>
      </div>
    </Tile>
  </div>
</template>
