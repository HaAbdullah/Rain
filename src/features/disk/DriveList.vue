<script setup lang="ts">
// The state before a result: the list of drives, or the progress of the scan that is running.
// A failed scan keeps the list and puts its error under the drive that failed.
import { computed } from 'vue'
import ErrorRow from '@/components/rain/ErrorRow.vue'
import ItemList from '@/components/rain/ItemList.vue'
import KeyHint from '@/components/rain/KeyHint.vue'
import ListRow from '@/components/rain/ListRow.vue'
import Meter from '@/components/rain/Meter.vue'
import { Button } from '@/components/ui/button'
import type { SelGroup } from '@/composables/useSelection'
import type { DiskScan, Drive, IconRef } from '@/data/types'
import { keepFocus } from '@/lib/focus'
import { formatBytes, formatNumber } from '@/lib/format'

const props = defineProps<{
  /** One group holding the drives, the same one the screen's selection walks. */
  groups: SelGroup<Drive>[]
  selectedId: string | null
  /** The newest scan, if any. Only a running or a failed one changes what is drawn. */
  scan: DiskScan | null
}>()

defineEmits<{
  'update:selectedId': [id: string]
  activate: [drive: Drive]
  context: [drive: Drive, event: MouseEvent]
  retry: []
  cancel: []
}>()

const DRIVE_ICON: IconRef = { kind: 'lucide', value: 'hard-drive' }

const scanningDrive = computed(() =>
  props.scan?.status === 'scanning'
    ? props.groups[0]?.items.find((drive) => drive.id === props.scan?.driveId)
    : undefined,
)

const title = (drive: Drive) => `${drive.letter}: ${drive.label}`
const usage = (drive: Drive) => `${formatBytes(drive.usedBytes)} of ${formatBytes(drive.totalBytes)} used`
</script>

<template>
  <div class="mx-auto flex h-full min-h-0 w-full max-w-[560px] flex-col pt-6">
    <div v-if="scanningDrive && scan" class="flex flex-col gap-3 p-2">
      <ListRow :icon="DRIVE_ICON" :title="title(scanningDrive)" subtitle="Scanning" status="running" selected size="lg" />
      <div class="flex flex-col gap-2 px-2">
        <Meter :value="scan.progress" tone="primary" />
        <div class="flex items-center justify-between">
          <span class="text-13 text-muted-foreground">{{ formatNumber(scan.filesSeen) }} files</span>
          <Button variant="ghost" size="sm" class="gap-2" tabindex="-1" @mousedown="keepFocus" @click="$emit('cancel')">
            Cancel scan
            <KeyHint keys="Ctrl+Enter" />
          </Button>
        </div>
      </div>
    </div>

    <ItemList
      v-else
      :groups="groups"
      :selected-id="selectedId"
      row-size="lg"
      @update:selected-id="$emit('update:selectedId', $event)"
      @activate="$emit('activate', $event)"
      @context="(drive, event) => $emit('context', drive, event)"
    >
      <template #row="{ item, selected }">
        <ListRow :icon="DRIVE_ICON" :title="title(item)" :subtitle="usage(item)" :selected="selected">
          <template #trailing>
            <Meter :value="item.usedBytes / item.totalBytes" :width="160" />
          </template>
        </ListRow>
        <!-- The row is inside the option: stop a double-click on Retry from also scanning the drive. -->
        <div v-if="scan?.status === 'failed' && scan.driveId === item.id" @dblclick.stop>
          <ErrorRow :message="scan.error ?? 'Scan failed'" @retry="$emit('retry')" />
        </div>
      </template>
    </ItemList>
  </div>
</template>
