<script setup lang="ts">
// The children of the open folder, biggest first: icon, name, size and the share of the folder.
import ItemList from '@/components/rain/ItemList.vue'
import Meter from '@/components/rain/Meter.vue'
import RIcon from '@/components/rain/RIcon.vue'
import type { SelGroup } from '@/composables/useSelection'
import type { DiskNode, FileCategory } from '@/data/types'
import { formatBytes } from '@/lib/format'
import { shareOf } from './folders'

export interface FolderRow {
  /** The node's path: unique inside a scan. */
  id: string
  node: DiskNode
}

defineProps<{
  /** One group holding the rows, the same one the screen's selection walks. */
  groups: SelGroup<FolderRow>[]
  selectedId: string | null
  /** The open folder, which the share bars compare with. */
  folder: DiskNode
}>()

defineEmits<{
  'update:selectedId': [id: string]
  activate: [row: FolderRow]
  context: [row: FolderRow, event: MouseEvent]
}>()

const ICON_BY_CATEGORY: Record<FileCategory, string> = {
  video: 'film',
  image: 'image',
  audio: 'music',
  archive: 'archive',
  code: 'file-code',
  document: 'file-text',
  app: 'app-window',
  other: 'file',
}

function iconOf(node: DiskNode) {
  const name =
    node.kind === 'other'
      ? 'ellipsis'
      : node.denied
        ? 'lock'
        : node.kind === 'dir'
          ? 'folder'
          : ICON_BY_CATEGORY[node.category ?? 'other']
  return { kind: 'lucide', value: name } as const
}
</script>

<template>
  <ItemList
    :groups="groups"
    :selected-id="selectedId"
    @update:selected-id="$emit('update:selectedId', $event)"
    @activate="$emit('activate', $event)"
    @context="(row, event) => $emit('context', row, event)"
  >
    <template #row="{ item, selected }">
      <div
        class="flex h-8 items-center gap-3 rounded-lg px-2 transition-colors"
        :class="selected ? 'bg-selected' : 'hover:bg-hover'"
      >
        <RIcon :icon="iconOf(item.node)" :size="16" class="shrink-0 text-muted-foreground" />
        <span class="min-w-0 flex-1 truncate text-sm text-foreground">{{ item.node.name }}</span>
        <span class="w-16 shrink-0 text-right font-mono text-xs text-muted-foreground">
          {{ item.node.denied ? '—' : formatBytes(item.node.sizeBytes) }}
        </span>
        <Meter :value="shareOf(item.node, folder)" :width="48" />
      </div>
    </template>
  </ItemList>
</template>
