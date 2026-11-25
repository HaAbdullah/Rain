<script setup lang="ts">
// One tile of the Hardware screen: a big icon, the name, and one to three short lines. The icon
// and the name are there at once; the lines fill in when the devices have been read.
import {
  BatteryChargingIcon,
  BatteryFullIcon,
  BatteryLowIcon,
  BatteryMediumIcon,
  BluetoothIcon,
  RadioIcon,
  TriangleAlertIcon,
  UsbIcon,
} from '@lucide/vue'
import { computed, type Component } from 'vue'
import Meter from '@/components/rain/Meter.vue'
import Tile from '@/components/rain/Tile.vue'
import { Skeleton } from '@/components/ui/skeleton'
import type { HardwareLink, HardwareTile } from '@/data/types'
import { lucideIcon } from '@/lib/icon-map'
import { driveShare, driveText, TILE_ICON, tileLines, visibleDrives, type BatteryLevel } from './model'

const props = defineProps<{
  tile: HardwareTile
  selected: boolean
  /** The devices have not been read yet: the lines show as bars. */
  loading: boolean
}>()

const BATTERY_ICON: Record<BatteryLevel, Component> = {
  charging: BatteryChargingIcon,
  full: BatteryFullIcon,
  medium: BatteryMediumIcon,
  low: BatteryLowIcon,
}

const LINK_ICON: Record<HardwareLink, Component> = {
  usb: UsbIcon,
  receiver: RadioIcon,
  bluetooth: BluetoothIcon,
}

const lines = computed(() => tileLines(props.tile))
const drives = computed(() => visibleDrives(props.tile))
const isDisk = computed(() => props.tile.kind === 'disk')

function iconOf(kind: string, level?: BatteryLevel): Component | null {
  if (kind === 'battery' && level) return BATTERY_ICON[level]
  if (kind === 'connection' && props.tile.connection) return LINK_ICON[props.tile.connection]
  if (kind === 'note') return TriangleAlertIcon
  return null
}
</script>

<template>
  <div class="h-56 min-h-0">
    <Tile :selected="selected" :dimmed="!loading && !tile.found && !isDisk">
      <div class="flex h-full min-h-0 flex-col items-center gap-3 overflow-hidden px-4 py-5 text-center">
        <component
          :is="lucideIcon(TILE_ICON[tile.kind])"
          :size="56"
          :stroke-width="1.5"
          class="shrink-0 text-foreground"
          aria-hidden="true"
        />
        <p class="max-w-full truncate text-base font-semibold text-foreground">{{ tile.name }}</p>

        <div v-if="loading" class="flex flex-col items-center gap-2 pt-1" aria-hidden="true">
          <Skeleton class="h-3 w-32" />
          <Skeleton class="h-3 w-24" />
        </div>

        <!-- The disk tile: the drives, fullest first, each with a thin bar. -->
        <div v-else-if="isDisk" class="flex w-full max-w-60 flex-col gap-2">
          <p v-if="drives.shown.length === 0" class="text-13 text-muted-foreground">
            {{ tile.problem ?? 'No drives found' }}
          </p>
          <div v-for="drive in drives.shown" :key="drive.id" class="flex flex-col gap-1">
            <p class="truncate font-mono text-xs text-muted-foreground">{{ driveText(drive) }}</p>
            <Meter :value="driveShare(drive)" />
          </div>
          <p v-if="drives.more > 0" class="text-xs text-muted-foreground">+{{ drives.more }} more</p>
        </div>

        <!-- A keyboard or a mouse: product, connection, battery. -->
        <div v-else class="flex w-full flex-col items-center gap-1">
          <p
            v-for="line in lines"
            :key="line.kind"
            class="flex max-w-full items-center gap-1.5 text-13"
            :class="line.kind === 'product' ? 'text-foreground' : 'text-muted-foreground'"
          >
            <component
              :is="iconOf(line.kind, line.level)"
              v-if="iconOf(line.kind, line.level)"
              :size="14"
              :stroke-width="1.75"
              class="shrink-0"
              aria-hidden="true"
            />
            <span class="truncate">{{ line.text }}</span>
          </p>
        </div>
      </div>
    </Tile>
  </div>
</template>
