<script setup lang="ts">
// The Disk screen (spec 7.7). Three states: the list of drives, a scan in progress, and
// the result, where the folder list and the treemap share one selection.
import { computed, watch } from 'vue'
import { runAction, runPrimary } from '@/actions/context'
import type { ActionTarget } from '@/actions/types'
import EmptyState from '@/components/rain/EmptyState.vue'
import ErrorRow from '@/components/rain/ErrorRow.vue'
import PathText from '@/components/rain/PathText.vue'
import ScreenFrame from '@/components/rain/ScreenFrame.vue'
import { Breadcrumb, BreadcrumbItem, BreadcrumbLink, BreadcrumbList, BreadcrumbPage, BreadcrumbSeparator } from '@/components/ui/breadcrumb'
import { Skeleton } from '@/components/ui/skeleton'
import { useActionPanel } from '@/composables/useActionPanel'
import { useResource } from '@/composables/useResource'
import { useScreen } from '@/composables/useScreen'
import { useSectionQuery } from '@/composables/useSearch'
import { useSelection, type SelGroup } from '@/composables/useSelection'
import { api } from '@/data'
import type { DiskNode, Drive } from '@/data/types'
import { keepFocus } from '@/lib/focus'
import { formatBytes } from '@/lib/format'
import { routeByName } from '@/nav/routes'
import { matches } from '@/lib/score'
import { cancelScanAction, diskView, scanAction } from './actions'
import { keepTreeWhileAway } from './diskCache'
import DriveList from './DriveList.vue'
import FolderList, { type FolderRow } from './FolderList.vue'
import { isInside, shareOf, trailTo } from './folders'
import TreemapCanvas from './TreemapCanvas.vue'

const { icon } = routeByName.disk

keepTreeWhileAway()

const filter = useSectionQuery('disk')
const drives = useResource('drives', () => api.listDrives())
const { scan, folderPath } = diskView

// ---- what the scan has produced ----

const scanning = computed(() => scan.value?.status === 'scanning')
const root = computed(() => (scan.value?.status === 'done' ? (scan.value.root ?? null) : null))
/** The root down to the open folder. The open folder is the last node. */
const trail = computed(() => (root.value ? trailTo(root.value, folderPath.value ?? root.value.path) : []))
const folder = computed(() => trail.value.at(-1) ?? null)

const rows = computed<FolderRow[]>(() =>
  (folder.value?.children ?? [])
    .filter((node) => matches(filter.value, node.name))
    .sort((a, b) => b.sizeBytes - a.sizeBytes)
    .map((node) => ({ id: node.path, node })),
)

// ---- selection: drives before a result, children of the open folder in it ----

const driveGroups = computed<SelGroup<Drive>[]>(() => [{ id: 'drives', items: drives.data.value ?? [] }])
const rowGroups = computed<SelGroup<FolderRow>[]>(() => [{ id: 'children', items: rows.value }])

const driveSelection = useSelection(driveGroups, { persistKey: 'disk.drive' })
const rowSelection = useSelection(rowGroups, { persistKey: 'disk.row', resetOn: filter })

// Opening a folder selects its first row; going up selects the folder just left.
// Sync, so the selection is right before the list is drawn.
watch(
  folder,
  (now, before) => {
    if (!now) return
    const cameFrom = before && now.children?.find((child) => isInside(before.path, child.path))
    const first = rows.value[0]
    if (cameFrom) rowSelection.select(cameFrom.path)
    else if (first) rowSelection.select(first.id)
  },
  { flush: 'sync' },
)

const selectedNode = computed(() => rowSelection.selected.value?.node ?? null)

const target = computed<ActionTarget | null>(() => {
  if (folder.value) {
    const node = selectedNode.value
    return node ? { type: 'diskNode', id: node.path, item: node } : null
  }
  // The running scan owns the screen, whichever drive row was selected before.
  const drive = scanning.value
    ? drives.data.value?.find((candidate) => candidate.id === scan.value?.driveId)
    : driveSelection.selected.value
  return drive ? { type: 'drive', id: drive.id, item: drive } : null
})

useScreen({
  target,
  move: (dir) => (folder.value ? rowSelection.move(dir) : driveSelection.move(dir)),
  page: (dir) => (folder.value ? rowSelection.page(dir) : driveSelection.page(dir)),
  stepOut() {
    const parent = trail.value.at(-2)
    if (!parent) return false
    folderPath.value = parent.path
    return true
  },
})

// ---- actions from the mouse ----

function targetOfRow(row: FolderRow): ActionTarget {
  return { type: 'diskNode', id: row.id, item: row.node }
}

function openPanel(row: FolderRow, event: MouseEvent) {
  rowSelection.select(row.id)
  useActionPanel().open(targetOfRow(row), event)
}

function openDrivePanel(drive: Drive, event: MouseEvent) {
  driveSelection.select(drive.id)
  useActionPanel().open({ type: 'drive', id: drive.id, item: drive }, event)
}

/** A double-click on the map does what Enter does on the row of its top-level folder. */
function activateOnMap(path: string) {
  const row = rows.value.find((candidate) => candidate.id === path)
  if (row) runPrimary(targetOfRow(row))
}

function sharePercent(node: DiskNode, parent: DiskNode): string {
  const percent = Math.round(shareOf(node, parent) * 100)
  return percent < 1 ? '<1%' : `${percent}%`
}
</script>

<template>
  <ScreenFrame>
    <template v-if="folder" #strip>
      <Breadcrumb class="min-w-0 flex-1 px-4">
        <BreadcrumbList class="flex-nowrap gap-1 text-13">
          <template v-for="(crumb, index) in trail" :key="crumb.path">
            <BreadcrumbSeparator v-if="index > 0" />
            <BreadcrumbItem class="min-w-0">
              <BreadcrumbPage v-if="index === trail.length - 1" class="truncate">{{ crumb.name }}</BreadcrumbPage>
              <BreadcrumbLink
                v-else
                as="button"
                type="button"
                tabindex="-1"
                class="truncate"
                @mousedown="keepFocus"
                @click="folderPath = crumb.path"
              >
                {{ crumb.name }}
              </BreadcrumbLink>
            </BreadcrumbItem>
          </template>
        </BreadcrumbList>
      </Breadcrumb>
      <span class="shrink-0 px-4 font-mono text-xs text-muted-foreground">{{ formatBytes(folder.sizeBytes) }}</span>
    </template>

    <!-- Result -->
    <template v-if="folder">
      <EmptyState v-if="!folder.children?.length" icon="folder-open" title="This folder is empty" />
      <div v-else class="grid min-h-0 flex-1 grid-cols-[minmax(0,5fr)_minmax(0,12fr)]">
        <div class="flex min-h-0 min-w-0 flex-col border-r border-border">
          <EmptyState v-if="rows.length === 0" icon="search-x" title="No match" />
          <div v-else class="min-h-0 flex-1">
            <FolderList
              :groups="rowGroups"
              :folder="folder"
              :selected-id="rowSelection.selectedId.value"
              @update:selected-id="rowSelection.select"
              @activate="runPrimary(targetOfRow($event))"
              @context="openPanel"
            />
          </div>
          <div v-if="selectedNode" class="flex shrink-0 flex-col gap-1 border-t border-border px-4 py-3">
            <PathText :path="selectedNode.path" />
            <p class="text-13 text-muted-foreground">
              {{ formatBytes(selectedNode.sizeBytes) }}, {{ sharePercent(selectedNode, folder) }} of {{ folder.name }}
            </p>
          </div>
        </div>
        <TreemapCanvas
          :folder="folder"
          :selected-path="selectedNode?.path ?? null"
          @select="rowSelection.select"
          @open="activateOnMap"
        />
      </div>
    </template>

    <!-- Before a result: drives, or a scan on its way -->
    <template v-else>
      <div v-if="drives.showSkeleton.value" class="mx-auto flex w-full max-w-[560px] flex-col gap-2 p-2 pt-6">
        <Skeleton v-for="n in 2" :key="n" class="h-14 w-full" />
      </div>
      <div v-else-if="drives.error.value && !drives.data.value" class="mx-auto w-full max-w-[560px] p-2 pt-6">
        <ErrorRow message="Drives unavailable" @retry="drives.reload()" />
      </div>
      <EmptyState v-else-if="drives.data.value?.length === 0" :icon="icon" title="No drives found" />
      <DriveList
        v-else-if="drives.data.value"
        :groups="driveGroups"
        :selected-id="driveSelection.selectedId.value"
        :scan="scan"
        @update:selected-id="driveSelection.select"
        @activate="runPrimary({ type: 'drive', id: $event.id, item: $event })"
        @context="openDrivePanel"
        @retry="scan && runAction(scanAction(scan.driveId))"
        @cancel="scan && runAction(cancelScanAction(scan))"
      />
    </template>
  </ScreenFrame>
</template>
