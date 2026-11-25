// The tile model of the Hardware screen: what a tile shows, as plain data. No Vue here, so
// the rules (which lines, which words) can be tested without a component.
import type { Drive, HardwareBattery, HardwareKind, HardwareLink, HardwareTile } from '@/data/types'
import { formatBytes } from '@/lib/format'

/** The Lucide icon of each kind of tile. */
export const TILE_ICON: Record<HardwareKind, string> = {
  disk: 'hard-drive',
  keyboard: 'keyboard',
  mouse: 'mouse',
}

const KIND_NAME: Record<HardwareKind, string> = {
  disk: 'Disk',
  keyboard: 'Keyboard',
  mouse: 'Mouse',
}

const LINK_NAME: Record<HardwareLink, string> = {
  usb: 'USB',
  receiver: '2.4 GHz receiver',
  bluetooth: 'Bluetooth',
}

/** The most drives a tile lists; the rest are counted. */
export const MAX_DRIVES = 3

/**
 * The tiles to draw before the first answer: name and icon only, so the screen is not empty
 * while the backend reads the devices.
 */
export function placeholderTiles(): HardwareTile[] {
  return (['disk', 'keyboard', 'mouse'] as const).map((kind) => ({
    id: kind,
    kind,
    name: KIND_NAME[kind],
    found: false,
    drives: [],
    opens: { kind: kind === 'disk' ? 'disk' : 'none' },
  }))
}

export function connectionLabel(link: HardwareLink): string {
  return LINK_NAME[link]
}

/** "85%", "about 85%" when the backend worked it out from a voltage. */
export function batteryText(battery: HardwareBattery): string {
  return `${battery.estimated ? 'about ' : ''}${battery.percent}%`
}

export type BatteryLevel = 'charging' | 'full' | 'medium' | 'low'

/** Which battery icon to draw. A battery that charges shows as charging, whatever its level. */
export function batteryLevel(battery: HardwareBattery): BatteryLevel {
  if (battery.charging) return 'charging'
  if (battery.percent >= 70) return 'full'
  if (battery.percent >= 30) return 'medium'
  return 'low'
}

/** "C: 120 GB free of 954 GB". */
export function driveText(drive: Drive): string {
  const free = Math.max(0, drive.totalBytes - drive.usedBytes)
  return `${drive.letter}: ${formatBytes(free)} free of ${formatBytes(drive.totalBytes)}`
}

/** How full a drive is, from 0 to 1. */
export function driveShare(drive: Drive): number {
  return drive.totalBytes > 0 ? Math.min(1, drive.usedBytes / drive.totalBytes) : 0
}

/** The drives to list and how many more there are. The fullest come first: they need a look. */
export function visibleDrives(tile: HardwareTile): { shown: Drive[]; more: number } {
  const byFill = [...tile.drives].sort((a, b) => driveShare(b) - driveShare(a))
  return { shown: byFill.slice(0, MAX_DRIVES), more: Math.max(0, byFill.length - MAX_DRIVES) }
}

/** One short line under the name of a tile. */
export interface TileLine {
  /** What the line is, for the icon in front of it. */
  kind: 'product' | 'connection' | 'battery' | 'note'
  text: string
  /** Set on a battery line: which icon to draw. */
  level?: BatteryLevel
}

/** The lines of a keyboard or mouse tile: at most three. The disk tile draws its drives instead. */
export function tileLines(tile: HardwareTile): TileLine[] {
  if (tile.kind === 'disk') return []
  if (!tile.found) {
    return [{ kind: 'note', text: tile.problem ?? `No ${KIND_NAME[tile.kind].toLowerCase()} found` }]
  }
  const lines: TileLine[] = []
  if (tile.product) lines.push({ kind: 'product', text: tile.product })
  if (tile.connection) lines.push({ kind: 'connection', text: connectionLabel(tile.connection) })
  if (tile.battery) {
    const charging = tile.battery.charging ? ', charging' : ''
    lines.push({ kind: 'battery', text: `${batteryText(tile.battery)}${charging}`, level: batteryLevel(tile.battery) })
  } else if (tile.problem) {
    lines.push({ kind: 'note', text: tile.problem })
  }
  return lines
}

/** What Enter does, in words for the action panel. */
export function openLabel(tile: HardwareTile): string {
  switch (tile.opens.kind) {
    case 'disk':
      return 'Open Disk'
    case 'none':
      return 'Nothing to open'
    default:
      return tile.opens.label ? `Open ${tile.opens.label}` : 'Open'
  }
}
