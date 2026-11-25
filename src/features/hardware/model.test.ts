import { describe, expect, it } from 'vitest'
import type { Drive, HardwareTile } from '@/data/types'
import {
  batteryLevel,
  batteryText,
  driveShare,
  driveText,
  MAX_DRIVES,
  openLabel,
  placeholderTiles,
  tileLines,
  visibleDrives,
} from './model'

function tile(over: Partial<HardwareTile> = {}): HardwareTile {
  return {
    id: 'mouse',
    kind: 'mouse',
    name: 'Mouse',
    found: true,
    drives: [],
    opens: { kind: 'none' },
    ...over,
  }
}

function drive(letter: string, total: number, used: number): Drive {
  return { id: `drive-${letter.toLowerCase()}`, letter, label: 'Local Disk', totalBytes: total, usedBytes: used }
}

const GB = 1024 ** 3

describe('placeholder tiles', () => {
  it('are Disk, Keyboard and Mouse with a name and nothing else', () => {
    const tiles = placeholderTiles()
    expect(tiles.map((t) => [t.id, t.name])).toEqual([
      ['disk', 'Disk'],
      ['keyboard', 'Keyboard'],
      ['mouse', 'Mouse'],
    ])
    expect(tiles.every((t) => t.drives.length === 0 && !t.found)).toBe(true)
    expect(tiles[0].opens.kind).toBe('disk')
  })
})

describe('tile lines', () => {
  it('give the product, the connection and the battery of a found device', () => {
    const lines = tileLines(
      tile({
        product: 'G305 Lightspeed',
        connection: 'receiver',
        battery: { percent: 20, charging: false, estimated: false },
      }),
    )
    expect(lines.map((l) => [l.kind, l.text])).toEqual([
      ['product', 'G305 Lightspeed'],
      ['connection', '2.4 GHz receiver'],
      ['battery', '20%'],
    ])
    expect(lines[2].level).toBe('low')
  })

  it('leave out what the device does not report', () => {
    const lines = tileLines(tile({ kind: 'keyboard', product: 'Board', connection: 'usb' }))
    expect(lines.map((l) => l.kind)).toEqual(['product', 'connection'])
  })

  it('say "charging" and "about" when the backend says so', () => {
    const lines = tileLines(tile({ battery: { percent: 64, charging: true, estimated: true } }))
    expect(lines[0].text).toBe('about 64%, charging')
    expect(lines[0].level).toBe('charging')
  })

  it('show one note for a device that was not found, or the problem when there is one', () => {
    expect(tileLines(tile({ found: false })).map((l) => l.text)).toEqual(['No mouse found'])
    expect(tileLines(tile({ found: false, problem: 'Logitech did not answer in time' })).map((l) => l.text)).toEqual([
      'Logitech did not answer in time',
    ])
  })

  it('never has more than three lines, and none for the disk tile (it draws its drives)', () => {
    const full = tile({
      product: 'x',
      connection: 'usb',
      battery: { percent: 50, estimated: false },
      problem: 'ignored while a battery is shown',
    })
    expect(tileLines(full)).toHaveLength(3)
    expect(tileLines(tile({ kind: 'disk', id: 'disk' }))).toEqual([])
  })
})

describe('battery', () => {
  it('has a level for the icon', () => {
    expect(batteryLevel({ percent: 90, estimated: false })).toBe('full')
    expect(batteryLevel({ percent: 50, estimated: false })).toBe('medium')
    expect(batteryLevel({ percent: 10, estimated: false })).toBe('low')
    expect(batteryLevel({ percent: 10, charging: true, estimated: false })).toBe('charging')
  })

  it('is written with its percent', () => {
    expect(batteryText({ percent: 87, estimated: false })).toBe('87%')
    expect(batteryText({ percent: 87, estimated: true })).toBe('about 87%')
  })
})

describe('drives', () => {
  it('are written as free of total', () => {
    expect(driveText(drive('C', 100 * GB, 40 * GB))).toBe('C: 60 GB free of 100 GB')
  })

  it('have a share from 0 to 1 and never divide by zero', () => {
    expect(driveShare(drive('C', 100, 25))).toBe(0.25)
    expect(driveShare(drive('C', 0, 0))).toBe(0)
    expect(driveShare(drive('C', 10, 50))).toBe(1)
  })

  it('list the fullest first, at most three, and count the rest', () => {
    const drives = [drive('C', 100, 10), drive('D', 100, 90), drive('E', 100, 50), drive('F', 100, 70), drive('G', 100, 20)]
    const { shown, more } = visibleDrives(tile({ kind: 'disk', drives }))
    expect(shown.map((d) => d.letter)).toEqual(['D', 'F', 'E'])
    expect(shown).toHaveLength(MAX_DRIVES)
    expect(more).toBe(2)
  })
})

describe('open label', () => {
  it('names what Enter does', () => {
    expect(openLabel(tile({ opens: { kind: 'disk', label: 'Disk' } }))).toBe('Open Disk')
    expect(openLabel(tile({ opens: { kind: 'app', label: 'Logitech G HUB' } }))).toBe('Open Logitech G HUB')
    expect(openLabel(tile({ opens: { kind: 'url' } }))).toBe('Open')
    expect(openLabel(tile())).toBe('Nothing to open')
  })
})
