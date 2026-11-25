// The model of a launch layout (section 02 3.4 of docs/backend): what opens with a project and
// where on the screen it goes. Pure functions, no Vue: the dialog and the diagram share them.
import type { AppEntry, IconRef, LayoutItem, LayoutStepOption, LayoutWhat, LayoutZone } from '@/data/types'

/** The most items a layout holds (the backend refuses more). */
export const MAX_ITEMS = 12

/** The monitor number that means "the monitor Rain's window is on". */
export const THIS_SCREEN = undefined

/** left, top, width, height as fractions of the work area of a monitor. */
export type Box = readonly [number, number, number, number]

export interface ZoneInfo {
  zone: LayoutZone
  label: string
  /** `null` for `free`: nothing is moved, so there is nothing to draw. */
  box: Box | null
}

const THIRD = 1 / 3

/** Every zone in the order that the picker lists them. */
export const ZONES: readonly ZoneInfo[] = [
  { zone: 'free', label: 'Where Windows puts it', box: null },
  { zone: 'full', label: 'Full screen', box: [0, 0, 1, 1] },
  { zone: 'left-half', label: 'Left half', box: [0, 0, 0.5, 1] },
  { zone: 'right-half', label: 'Right half', box: [0.5, 0, 0.5, 1] },
  { zone: 'top-half', label: 'Top half', box: [0, 0, 1, 0.5] },
  { zone: 'bottom-half', label: 'Bottom half', box: [0, 0.5, 1, 0.5] },
  { zone: 'top-left', label: 'Top left', box: [0, 0, 0.5, 0.5] },
  { zone: 'top-right', label: 'Top right', box: [0.5, 0, 0.5, 0.5] },
  { zone: 'bottom-left', label: 'Bottom left', box: [0, 0.5, 0.5, 0.5] },
  { zone: 'bottom-right', label: 'Bottom right', box: [0.5, 0.5, 0.5, 0.5] },
  { zone: 'left-third', label: 'Left third', box: [0, 0, THIRD, 1] },
  { zone: 'middle-third', label: 'Middle third', box: [THIRD, 0, THIRD, 1] },
  { zone: 'right-third', label: 'Right third', box: [2 * THIRD, 0, THIRD, 1] },
  { zone: 'left-two-thirds', label: 'Left two thirds', box: [0, 0, 2 * THIRD, 1] },
  { zone: 'right-two-thirds', label: 'Right two thirds', box: [THIRD, 0, 2 * THIRD, 1] },
]

export function zoneInfo(zone: LayoutZone): ZoneInfo {
  return ZONES.find((info) => info.zone === zone) ?? ZONES[0]
}

export function zoneLabel(zone: LayoutZone): string {
  return zoneInfo(zone).label
}

// ---- what an item is ----

export function whatKey(what: LayoutWhat): string {
  if (what.kind === 'step') return `step:${what.name}`
  if (what.kind === 'app') return `app:${what.id}`
  return 'folder'
}

export interface Described {
  label: string
  icon: IconRef
}

const FOLDER_ICON: IconRef = { kind: 'lucide', value: 'folder-open' }
const APP_ICON: IconRef = { kind: 'lucide', value: 'app-window' }
const UNKNOWN_ICON: IconRef = { kind: 'lucide', value: 'circle-help' }

/** The name and icon of an item. A step or an app that is gone is shown by its name, so it can be removed. */
export function describe(what: LayoutWhat, steps: readonly LayoutStepOption[], apps: readonly AppEntry[]): Described {
  if (what.kind === 'folder') return { label: 'Project folder', icon: FOLDER_ICON }
  if (what.kind === 'step') {
    const step = steps.find((option) => option.name === what.name)
    return step ? { label: step.label, icon: step.icon } : { label: what.name, icon: UNKNOWN_ICON }
  }
  const app = apps.find((entry) => entry.id === what.id)
  return app ? { label: app.name, icon: app.icon } : { label: 'Unknown app', icon: APP_ICON }
}

// ---- editing a list of items ----

/** The zones that a new item tries first, in this order, so that adding items fills the screen. */
const SUGGESTED: readonly LayoutZone[] = [
  'right-half',
  'left-half',
  'top-right',
  'bottom-right',
  'top-left',
  'bottom-left',
  'full',
]

export function canAdd(items: readonly LayoutItem[]): boolean {
  return items.length < MAX_ITEMS
}

/** Adds an item in the first suggested zone that no item of the same monitor uses; `free` when all are used. */
export function addItem(items: readonly LayoutItem[], what: LayoutWhat, monitor?: number): LayoutItem[] {
  if (!canAdd(items)) return [...items]
  const used = new Set(items.filter((item) => item.monitor === monitor).map((item) => item.zone))
  const zone = SUGGESTED.find((candidate) => !used.has(candidate)) ?? 'free'
  return [...items, { what, zone, ...(monitor === undefined ? {} : { monitor }) }]
}

export function removeItem(items: readonly LayoutItem[], index: number): LayoutItem[] {
  return items.filter((_, i) => i !== index)
}

export function setZone(items: readonly LayoutItem[], index: number, zone: LayoutZone): LayoutItem[] {
  return items.map((item, i) => (i === index ? { ...item, zone } : item))
}

/** `undefined` puts the item on the monitor of Rain's window. */
export function setMonitor(items: readonly LayoutItem[], index: number, monitor: number | undefined): LayoutItem[] {
  return items.map((item, i) => {
    if (i !== index) return item
    const { monitor: _old, ...rest } = item
    return monitor === undefined ? rest : { ...rest, monitor }
  })
}

export function sameItems(a: readonly LayoutItem[], b: readonly LayoutItem[]): boolean {
  return a.length === b.length && a.every((item, i) => JSON.stringify(item) === JSON.stringify(b[i]))
}

// ---- presets ----

export interface Preset {
  id: string
  label: string
  /** The icons of the picture on the button. */
  icon: string
  items: readonly LayoutItem[]
}

const step = (name: string, zone: LayoutZone): LayoutItem => ({ what: { kind: 'step', name }, zone })

/** The four starting points of the editor. Each one only names built-in steps. */
export const PRESETS: readonly Preset[] = [
  {
    id: 'side-by-side',
    label: 'Editor and terminal side by side',
    icon: 'columns-2',
    items: [step('editor', 'right-half'), step('terminal', 'left-half')],
  },
  {
    id: 'stacked',
    label: 'Editor left, terminal and Claude stacked right',
    icon: 'panel-left',
    items: [step('editor', 'left-half'), step('terminal', 'top-right'), step('claude', 'bottom-right')],
  },
  {
    id: 'quadrants',
    label: 'Four quadrants',
    icon: 'layout-grid',
    items: [
      step('editor', 'top-left'),
      step('terminal', 'top-right'),
      step('claude', 'bottom-left'),
      { what: { kind: 'folder' }, zone: 'bottom-right' },
    ],
  },
  {
    id: 'free',
    label: 'Everything where Windows puts it',
    icon: 'app-window',
    items: [step('editor', 'free'), step('claude', 'free')],
  },
]

/** The preset that the items are exactly, if any. */
export function presetOf(items: readonly LayoutItem[]): Preset | undefined {
  return PRESETS.find((preset) => sameItems(preset.items, items))
}

/** A copy of a preset's items. Items of a preset that the project cannot open (an unknown step) are left out. */
export function itemsOfPreset(preset: Preset, steps: readonly LayoutStepOption[]): LayoutItem[] {
  const openable = (what: LayoutWhat) =>
    what.kind !== 'step' || steps.some((option) => option.name === what.name)
  return preset.items.filter((item) => openable(item.what)).map((item) => ({ ...item }))
}

// ---- the picture ----

export interface DiagramZone {
  zone: LayoutZone
  box: Box
  items: LayoutItem[]
}

export interface DiagramScreen {
  /** `undefined`: the monitor of Rain's window. */
  monitor: number | undefined
  zones: DiagramZone[]
}

export interface Diagram {
  screens: DiagramScreen[]
  /** Items that open where Windows puts them. */
  free: LayoutItem[]
}

/** The screens with their zones in use, the items that are not placed, in the order the items stand. */
export function diagramOf(items: readonly LayoutItem[]): Diagram {
  const free = items.filter((item) => item.zone === 'free')
  const screens: DiagramScreen[] = []
  for (const item of items) {
    const box = zoneInfo(item.zone).box
    if (!box) continue
    let screen = screens.find((s) => s.monitor === item.monitor)
    if (!screen) {
      screen = { monitor: item.monitor, zones: [] }
      screens.push(screen)
    }
    const zone = screen.zones.find((z) => z.zone === item.zone)
    if (zone) zone.items.push(item)
    else screen.zones.push({ zone: item.zone, box, items: [item] })
  }
  // "This screen" first, then the numbered monitors from the left.
  screens.sort((a, b) => (a.monitor ?? 0) - (b.monitor ?? 0))
  return { screens, free }
}

export function screenLabel(monitor: number | undefined): string {
  return monitor === undefined ? 'This screen' : `Monitor ${monitor}`
}

/** What the quiet line after a launch says: the steps that started but whose window was not placed. */
export function placementNotes(steps: readonly { label: string; note?: string }[]): string[] {
  return steps.flatMap((step) => (step.note ? [`${step.label}: ${step.note}`] : []))
}
