<script setup lang="ts">
// A small diagram of the screen: each zone that the layout uses is a box with the icons of what
// goes there. One screen for "this screen", one more for each numbered monitor. Items that open
// where Windows puts them are listed under the screens.
import { computed } from 'vue'
import RIcon from '@/components/rain/RIcon.vue'
import type { AppEntry, LayoutItem, LayoutStepOption } from '@/data/types'
import { describe, diagramOf, screenLabel, zoneLabel } from './layout'

const props = defineProps<{
  items: readonly LayoutItem[]
  steps: readonly LayoutStepOption[]
  apps: readonly AppEntry[]
}>()

const diagram = computed(() => diagramOf(props.items))

function percent(fraction: number): string {
  return `${(fraction * 100).toFixed(3)}%`
}

function describeAll(items: readonly LayoutItem[]) {
  return items.map((item) => describe(item.what, props.steps, props.apps))
}

function ariaOf(monitor: number | undefined): string {
  const screen = diagram.value.screens.find((s) => s.monitor === monitor)
  if (!screen) return ''
  const parts = screen.zones.map(
    (zone) => `${zoneLabel(zone.zone)}: ${describeAll(zone.items).map((d) => d.label).join(', ')}`,
  )
  return `${screenLabel(monitor)}. ${parts.join('. ')}`
}
</script>

<template>
  <div class="flex flex-col gap-2">
    <div class="flex gap-3">
      <div
        v-for="screen in diagram.screens"
        :key="screen.monitor ?? 'this'"
        class="flex min-w-0 flex-1 flex-col gap-1"
      >
        <span v-if="diagram.screens.length > 1" class="truncate text-xs text-muted-foreground">
          {{ screenLabel(screen.monitor) }}
        </span>
        <div
          class="relative aspect-video w-full overflow-hidden rounded-md border border-border bg-surface-0"
          role="img"
          :aria-label="ariaOf(screen.monitor)"
        >
          <div
            v-for="zone in screen.zones"
            :key="zone.zone"
            class="absolute"
            :style="{
              left: percent(zone.box[0]),
              top: percent(zone.box[1]),
              width: percent(zone.box[2]),
              height: percent(zone.box[3]),
            }"
          >
            <div
              class="absolute inset-px flex flex-wrap content-center items-center justify-center gap-1 overflow-hidden rounded-sm bg-selected"
              :title="zoneLabel(zone.zone)"
            >
              <RIcon
                v-for="(described, index) in describeAll(zone.items)"
                :key="index"
                :icon="described.icon"
                :size="16"
                :label="described.label"
              />
            </div>
          </div>
        </div>
      </div>
    </div>

    <div v-if="diagram.free.length > 0" class="flex items-center gap-2 text-xs text-muted-foreground">
      <span class="shrink-0">Where Windows puts them</span>
      <RIcon
        v-for="(described, index) in describeAll(diagram.free)"
        :key="index"
        :icon="described.icon"
        :size="14"
        :label="described.label"
      />
    </div>
  </div>
</template>
