import { describe, expect, it } from 'vitest'
import type { AppEntry, LayoutItem, LayoutStepOption, LayoutZone } from '@/data/types'
import {
  MAX_ITEMS,
  PRESETS,
  ZONES,
  addItem,
  describe as describeItem,
  diagramOf,
  itemsOfPreset,
  placementNotes,
  presetOf,
  removeItem,
  sameItems,
  screenLabel,
  setMonitor,
  setZone,
  whatKey,
  zoneInfo,
} from './layout'

const steps: LayoutStepOption[] = [
  { name: 'editor', kind: 'editor', label: 'VS Code', icon: { kind: 'lucide', value: 'code-xml' } },
  { name: 'terminal', kind: 'terminal', label: 'Terminal', icon: { kind: 'lucide', value: 'square-terminal' } },
  { name: 'claude', kind: 'claude', label: 'Claude Code', icon: { kind: 'brand', value: 'claude' } },
]

const editor = (zone: LayoutZone, monitor?: number): LayoutItem => ({
  what: { kind: 'step', name: 'editor' },
  zone,
  ...(monitor === undefined ? {} : { monitor }),
})

describe('zones', () => {
  it('has the fifteen zones of the backend, each once, and free is the only one without a box', () => {
    const names = ZONES.map((info) => info.zone)
    expect(new Set(names).size).toBe(15)
    expect(names[0]).toBe('free')
    expect(ZONES.filter((info) => info.box === null).map((info) => info.zone)).toEqual(['free'])
  })

  it('draws halves, quadrants and thirds that tile the screen without a gap', () => {
    const [lx, , lw] = zoneInfo('left-half').box!
    const [rx, , rw] = zoneInfo('right-half').box!
    expect(lx + lw).toBeCloseTo(rx)
    expect(rx + rw).toBeCloseTo(1)
    const [, ty, , th] = zoneInfo('top-half').box!
    const [, by] = zoneInfo('bottom-half').box!
    expect(ty + th).toBeCloseTo(by)
    const thirds = (['left-third', 'middle-third', 'right-third'] as const).map((zone) => zoneInfo(zone).box!)
    expect(thirds[0][0] + thirds[0][2]).toBeCloseTo(thirds[1][0])
    expect(thirds[1][0] + thirds[1][2]).toBeCloseTo(thirds[2][0])
    expect(thirds[2][0] + thirds[2][2]).toBeCloseTo(1)
    const [, , w] = zoneInfo('left-two-thirds').box!
    expect(w).toBeCloseTo(2 / 3)
  })
})

describe('items', () => {
  it('adds an item in the first suggested zone that the monitor does not use yet', () => {
    let items: LayoutItem[] = []
    items = addItem(items, { kind: 'step', name: 'editor' })
    items = addItem(items, { kind: 'step', name: 'terminal' })
    expect(items.map((item) => item.zone)).toEqual(['right-half', 'left-half'])
    // Another monitor starts again from the first suggestion.
    items = addItem(items, { kind: 'folder' }, 2)
    expect(items[2]).toEqual({ what: { kind: 'folder' }, zone: 'right-half', monitor: 2 })
  })

  it('falls back to free when every suggestion is taken, and stops at the limit', () => {
    let items: LayoutItem[] = []
    for (let i = 0; i < 7; i += 1) items = addItem(items, { kind: 'step', name: 'editor' })
    expect(addItem(items, { kind: 'folder' }).at(-1)?.zone).toBe('free')
    for (let i = 0; i < 10; i += 1) items = addItem(items, { kind: 'folder' })
    expect(items).toHaveLength(MAX_ITEMS)
  })

  it('changes one item and leaves the list it was given alone', () => {
    const items = [editor('left-half'), editor('right-half', 2)]
    expect(setZone(items, 0, 'full')[0].zone).toBe('full')
    expect(items[0].zone).toBe('left-half')
    expect(setMonitor(items, 0, 3)[0].monitor).toBe(3)
    expect('monitor' in setMonitor(items, 1, undefined)[1]).toBe(false)
    expect(removeItem(items, 0)).toEqual([items[1]])
  })

  it('compares lists by value', () => {
    expect(sameItems([editor('full')], [editor('full')])).toBe(true)
    expect(sameItems([editor('full')], [editor('full', 1)])).toBe(false)
    expect(sameItems([editor('full')], [])).toBe(false)
  })

  it('names an item by what it opens', () => {
    expect(whatKey({ kind: 'step', name: 'editor' })).toBe('step:editor')
    expect(whatKey({ kind: 'app', id: 'app-x' })).toBe('app:app-x')
    expect(whatKey({ kind: 'folder' })).toBe('folder')
  })

  it('describes steps, apps and the folder, and does not fail on something that is gone', () => {
    const apps = [{ id: 'app-notepad', name: 'Notepad', icon: { kind: 'lucide', value: 'file' } }] as AppEntry[]
    expect(describeItem({ kind: 'step', name: 'claude' }, steps, apps).label).toBe('Claude Code')
    expect(describeItem({ kind: 'app', id: 'app-notepad' }, steps, apps).label).toBe('Notepad')
    expect(describeItem({ kind: 'folder' }, steps, apps).label).toBe('Project folder')
    expect(describeItem({ kind: 'step', name: 'gone' }, steps, apps).label).toBe('gone')
    expect(describeItem({ kind: 'app', id: 'app-gone' }, steps, apps).label).toBe('Unknown app')
  })
})

describe('presets', () => {
  it('has the four presets of the spec', () => {
    expect(PRESETS.map((preset) => preset.label)).toEqual([
      'Editor and terminal side by side',
      'Editor left, terminal and Claude stacked right',
      'Four quadrants',
      'Everything where Windows puts it',
    ])
  })

  it('puts no two items of a preset in the same zone, except where Windows puts them', () => {
    for (const preset of PRESETS) {
      const zones = preset.items.map((item) => item.zone).filter((zone) => zone !== 'free')
      expect(new Set(zones).size).toBe(zones.length)
      expect(preset.items.length).toBeLessThanOrEqual(MAX_ITEMS)
    }
  })

  it('stacks the terminal over Claude on the right of the editor', () => {
    const stacked = PRESETS[1].items
    expect(stacked.map((item) => [whatKey(item.what), item.zone])).toEqual([
      ['step:editor', 'left-half'],
      ['step:terminal', 'top-right'],
      ['step:claude', 'bottom-right'],
    ])
  })

  it('is found again from its items, and gives copies', () => {
    const items = itemsOfPreset(PRESETS[0], steps)
    expect(presetOf(items)?.id).toBe('side-by-side')
    items[0].zone = 'full'
    expect(PRESETS[0].items[0].zone).toBe('right-half')
    expect(presetOf(items)).toBeUndefined()
  })

  it('leaves out a step that the project cannot open', () => {
    const onlyEditor = steps.slice(0, 1)
    expect(itemsOfPreset(PRESETS[1], onlyEditor).map((item) => whatKey(item.what))).toEqual(['step:editor'])
  })
})

describe('the diagram', () => {
  it('groups items by monitor and zone, "this screen" first, and sets free items apart', () => {
    const items: LayoutItem[] = [
      editor('left-half', 2),
      editor('right-half'),
      { what: { kind: 'folder' }, zone: 'left-half', monitor: 2 },
      editor('free'),
      editor('top-left', 1),
    ]
    const diagram = diagramOf(items)
    expect(diagram.screens.map((screen) => screen.monitor)).toEqual([undefined, 1, 2])
    const second = diagram.screens[2]
    expect(second.zones).toHaveLength(1)
    expect(second.zones[0].items).toHaveLength(2)
    expect(diagram.free).toEqual([editor('free')])
  })

  it('is empty for a list of items that are all free', () => {
    const diagram = diagramOf([editor('free')])
    expect(diagram.screens).toEqual([])
    expect(diagram.free).toHaveLength(1)
  })

  it('labels the screens', () => {
    expect(screenLabel(undefined)).toBe('This screen')
    expect(screenLabel(2)).toBe('Monitor 2')
  })
})

describe('the line after a launch', () => {
  it('names the steps that could not be placed and says nothing otherwise', () => {
    expect(placementNotes([{ label: 'VS Code' }, { label: 'Terminal', note: 'Its window did not show up' }])).toEqual([
      'Terminal: Its window did not show up',
    ])
    expect(placementNotes([{ label: 'VS Code' }])).toEqual([])
  })
})
