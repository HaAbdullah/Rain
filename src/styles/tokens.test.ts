// The themes are sets of the same tokens (src/styles/tokens.css). These tests read the file
// and check that every theme defines every token, that the swatch colours of the picker are
// the real colours, and that the inline script of index.html knows the same themes.
import { readFileSync } from 'node:fs'
import { describe, expect, it } from 'vitest'
import { themes } from '@/composables/useTheme'

// Vitest runs from the project folder. A CSS import would come back empty, so the file is read as text.
const tokensCss = readFileSync('src/styles/tokens.css', 'utf8')
const indexHtml = readFileSync('index.html', 'utf8')
const css = tokensCss.replace(/\/\*[\s\S]*?\*\//g, '')

interface Block {
  selector: string
  body: string
  tokens: Map<string, string>
}

const BLOCK = /(?:^|\n)([^{}@]*?data-theme[^{}]*)\{([^{}]*)\}/g

/** The top-level rules of the file whose selector mentions data-theme, with their custom properties. */
function themeBlocks(): Map<string, Block> {
  const blocks = new Map<string, Block>()
  for (const match of css.matchAll(BLOCK)) {
    const selector = match[1]!.trim()
    const body = match[2]!
    const id = /data-theme="([^"]+)"/.exec(selector)?.[1]
    if (!id) continue
    const tokens = new Map<string, string>()
    for (const declaration of body.matchAll(/--([\w-]+)\s*:\s*([^;]+);/g)) {
      tokens.set(declaration[1]!, declaration[2]!.trim())
    }
    blocks.set(id, { selector, body, tokens })
  }
  return blocks
}

const blocks = themeBlocks()
const ids = themes.map((entry) => entry.id)

describe('tokens.css themes', () => {
  it('has one block for each theme of useTheme and no other', () => {
    expect([...blocks.keys()].sort()).toEqual([...ids].sort())
  })

  it('makes Dark the default: its block also answers to a plain :root', () => {
    expect(blocks.get('dark')?.selector).toMatch(/^:root\s*,/)
  })

  it('defines every token in every theme', () => {
    const dark = blocks.get('dark')!.tokens
    expect(dark.size).toBeGreaterThan(60)
    for (const id of ids) {
      const tokens = blocks.get(id)!.tokens
      const missing = [...dark.keys()].filter((name) => !tokens.has(name))
      const extra = [...tokens.keys()].filter((name) => !dark.has(name))
      expect({ id, missing, extra }).toEqual({ id, missing: [], extra: [] })
    }
  })

  it('covers status, heat, chart, file, scrollbar, selection and shadow colours', () => {
    const names = [...blocks.get('dark')!.tokens.keys()]
    for (const prefix of ['status-', 'heat-', 'chart-', 'file-', 'scrollbar-', 'selection-', 'shadow-']) {
      expect(names.some((name) => name.startsWith(prefix))).toBe(true)
    }
    for (const n of [0, 1, 2, 3, 4]) expect(names).toContain(`heat-${n}`)
  })

  it('gives every token a value of its own, so a theme never inherits one by accident', () => {
    for (const [id, { tokens }] of blocks) {
      for (const [name, value] of tokens) expect(value, `${id} ${name}`).not.toBe('')
    }
  })

  it('sets color-scheme: light for Light and dark for the rest', () => {
    for (const id of ids) {
      expect(blocks.get(id)!.body).toContain(`color-scheme: ${id === 'light' ? 'light' : 'dark'};`)
    }
  })

  it('draws the picker swatches in the colours of the tokens', () => {
    for (const entry of themes) {
      const tokens = blocks.get(entry.id)!.tokens
      expect(entry.swatch.surface, `${entry.id} surface`).toBe(tokens.get('background'))
      expect(entry.swatch.text, `${entry.id} text`).toBe(tokens.get('foreground'))
      expect(entry.swatch.accent, `${entry.id} accent`).toBe(tokens.get('primary'))
    }
  })

  it('keeps no hex colour outside the theme blocks', () => {
    let outside = css
    for (const match of css.matchAll(BLOCK)) outside = outside.replace(match[0], '')
    expect(outside.match(/#[0-9a-fA-F]{3,8}\b/g) ?? []).toEqual([])
  })
})

// ---- contrast (WCAG 2.x): 4.5:1 for text, 3:1 for icons and borders that carry meaning ----

function luminance(hex: string): number {
  const [r, g, b] = [1, 3, 5].map((at) => {
    const value = parseInt(hex.slice(at, at + 2), 16) / 255
    return value <= 0.03928 ? value / 12.92 : ((value + 0.055) / 1.055) ** 2.4
  })
  return 0.2126 * r! + 0.7152 * g! + 0.0722 * b!
}

function contrast(a: string, b: string): number {
  const [x, y] = [luminance(a), luminance(b)]
  return (Math.max(x, y) + 0.05) / (Math.min(x, y) + 0.05)
}

const SURFACES = ['background', 'surface-0', 'surface-1', 'surface-2', 'hover', 'selected']
const PLAIN_SURFACES = SURFACES.filter((name) => name !== 'selected')
const WELLS = ['background', 'surface-0', 'surface-1']

describe.each(ids)('contrast in %s', (id) => {
  const tokens = blocks.get(id)!.tokens
  const color = (name: string) => tokens.get(name)!

  /** The lowest ratio of `name` over `onto`, and where it is. */
  function worst(name: string, onto: string[]) {
    return onto.map((surface) => ({ surface, ratio: contrast(color(name), color(surface)) })).sort((a, b) => a.ratio - b.ratio)[0]!
  }
  function expectAtLeast(minimum: number, name: string, onto: string[]) {
    const found = worst(name, onto)
    expect(found.ratio, `${name} on ${found.surface}`).toBeGreaterThanOrEqual(minimum)
  }

  it('keeps text readable on every surface', () => {
    for (const name of ['foreground', 'muted-foreground', 'text-3']) expectAtLeast(4.5, name, SURFACES)
    expectAtLeast(4.5, 'sidebar-foreground', ['surface-1', 'hover', 'selected'])
    expectAtLeast(4.5, 'destructive', ['background', 'surface-1', 'surface-2', 'hover', 'selected'])
  })

  it('keeps the status colours readable, and visible on the selected row', () => {
    for (const name of ['running', 'waiting', 'success', 'failed', 'idle']) {
      expectAtLeast(4.5, `status-${name}`, PLAIN_SURFACES)
      expectAtLeast(3, `status-${name}`, ['selected'])
    }
  })

  it('keeps focus, input borders and icon tints visible', () => {
    expectAtLeast(3, 'ring', SURFACES)
    expectAtLeast(3, 'border-strong', ['background', 'surface-1', 'surface-2'])
    for (const n of [1, 2, 3, 4, 5]) expectAtLeast(3, `chart-${n}`, SURFACES)
  })

  it('keeps the text on filled colours readable', () => {
    expect(contrast(color('primary-foreground'), color('primary'))).toBeGreaterThanOrEqual(4.5)
    expect(contrast(color('destructive-foreground'), color('destructive'))).toBeGreaterThanOrEqual(4.5)
    expect(contrast(color('selection-fg'), color('selection-bg'))).toBeGreaterThanOrEqual(4.5)
    expect(contrast(color('on-heat'), color('heat-3'))).toBeGreaterThanOrEqual(4.5)
    for (const category of ['video', 'image', 'audio', 'archive', 'code', 'document', 'app', 'other']) {
      expect(contrast(color('on-file'), color(`file-${category}`)), `on-file on file-${category}`).toBeGreaterThanOrEqual(4.5)
      expectAtLeast(3, `file-${category}`, ['surface-0'])
    }
  })

  it('shows every heatmap step, including the empty one', () => {
    expectAtLeast(1.3, 'heat-0', WELLS)
    for (const n of [1, 2, 3, 4]) expectAtLeast(3, `heat-${n}`, WELLS)
  })
})

describe('index.html', () => {
  it('applies a stored theme from the same list before the first paint', () => {
    const script = /<script>([\s\S]*?)<\/script>/.exec(indexHtml)?.[1] ?? ''
    expect(script).toContain("'rain.theme'")
    const listed = /\[([^\]]*)\]\.indexOf/.exec(script)?.[1] ?? ''
    expect([...listed.matchAll(/'([\w-]+)'/g)].map((match) => match[1])).toEqual(ids)
  })
})
