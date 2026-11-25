<script setup lang="ts">
// The programs that game mode would close on the whole PC (spec 7.8): one row each with icon,
// name, memory and a tick. Unticking keeps the program; the backend remembers it in gamemode.toml.
import { computed } from 'vue'
import RIcon from '@/components/rain/RIcon.vue'
import { Checkbox } from '@/components/ui/checkbox'
import { Switch } from '@/components/ui/switch'
import type { OtherApps } from '@/data/types'
import { formatBytes } from '@/lib/format'
import { lucideIcon } from '@/lib/icon-map'

const props = defineProps<{
  list: OtherApps
  /** False once the countdown is over: the list is only read. */
  editable: boolean
}>()

const emit = defineEmits<{
  /** The master switch. */
  enabled: [value: boolean]
  /** One program: true closes it, false keeps it. */
  close: [id: string, value: boolean]
}>()

const closing = computed(() => props.list.apps.filter((app) => app.close))
const summary = computed(() => {
  if (!props.list.enabled) return 'Only Rain is closed'
  if (closing.value.length === 0) return 'Nothing to close'
  const n = closing.value.length
  return `${n} ${n === 1 ? 'app' : 'apps'} · frees ${formatBytes(props.list.freedBytes)}`
})
</script>

<template>
  <section class="flex flex-col gap-2" aria-label="Other apps">
    <div class="flex h-(--row-h) items-center gap-3 rounded-lg border border-border bg-surface-1 px-3">
      <component :is="lucideIcon('layout-grid')" :size="20" :stroke-width="1.75" class="shrink-0 text-muted-foreground" aria-hidden="true" />
      <span class="min-w-0 flex-1 truncate text-sm text-foreground">Close other apps</span>
      <span class="text-13 text-muted-foreground tabular-nums">{{ summary }}</span>
      <Switch
        :model-value="list.enabled"
        :disabled="!editable"
        aria-label="Close other apps"
        tabindex="-1"
        @update:model-value="(value: boolean) => emit('enabled', value)"
      />
    </div>

    <ul
      v-if="list.enabled && list.apps.length > 0"
      class="flex max-h-72 flex-col divide-y divide-border overflow-y-auto rounded-lg border border-border bg-surface-1"
    >
      <li v-for="app in list.apps" :key="app.id">
        <label class="flex h-(--row-h) cursor-pointer items-center gap-3 px-3" :class="{ 'opacity-60': !app.close }">
          <RIcon :icon="app.icon" :size="20" :label="app.name" />
          <span class="min-w-0 flex-1 truncate text-sm text-foreground">{{ app.name }}</span>
          <component
            :is="lucideIcon('app-window')"
            v-if="app.hasWindow"
            :size="14"
            :stroke-width="1.75"
            class="shrink-0 text-text-3"
            aria-label="Has a window; it is asked to close"
          />
          <span class="w-16 text-right text-13 text-muted-foreground tabular-nums">{{ formatBytes(app.memoryBytes) }}</span>
          <Checkbox
            :model-value="app.close"
            :disabled="!editable"
            :aria-label="`Close ${app.name}`"
            tabindex="-1"
            @update:model-value="(value: boolean | 'indeterminate') => emit('close', app.id, value === true)"
          />
        </label>
      </li>
    </ul>

    <p v-if="list.enabled && list.alwaysKept > 0" class="flex items-center justify-center gap-1.5 text-13 text-text-3">
      <component :is="lucideIcon('shield-check')" :size="14" :stroke-width="1.75" aria-hidden="true" />
      {{ list.alwaysKept }} always kept
    </p>
  </section>
</template>
