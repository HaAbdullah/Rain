<script setup lang="ts">
// A project's icon, name and state marker (folder missing or open), shared by both card sizes.
import { FolderXIcon } from '@lucide/vue'
import RIcon from '@/components/rain/RIcon.vue'
import type { Project } from '@/data/types'

defineProps<{
  project: Project
  iconSize: number
  /** Type classes of the name. */
  nameClass: string
  /** Let the name use two lines instead of one (the large card). */
  wrap?: boolean
}>()
</script>

<template>
  <RIcon :icon="project.icon" :size="iconSize" />
  <span
    class="min-w-0 text-foreground"
    :class="[wrap ? 'line-clamp-2 break-words leading-tight' : 'truncate', nameClass]"
    :title="project.name"
  >{{ project.name }}</span>
  <FolderXIcon
    v-if="project.missing"
    :size="16"
    :stroke-width="1.75"
    class="shrink-0 text-muted-foreground"
    aria-label="Folder not found"
  />
  <span
    v-else-if="project.open"
    class="size-2 shrink-0 rounded-full bg-status-success"
    role="img"
    aria-label="Open"
    title="Open"
  />
</template>
