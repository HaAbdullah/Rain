import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

/** A stand-in for the page: a root element that records data-theme and the dark class. */
function stubPage(stored: Record<string, string> = {}) {
  const store = new Map(Object.entries(stored))
  const classes = new Set<string>()
  const root = {
    dataset: {} as Record<string, string>,
    classList: {
      toggle: (name: string, on: boolean) => (on ? classes.add(name) : classes.delete(name)),
      contains: (name: string) => classes.has(name),
    },
  }
  // Vue's runtime reads createElement when it loads, so the stub has one.
  vi.stubGlobal('document', { documentElement: root, createElement: () => ({}) })
  vi.stubGlobal('localStorage', {
    getItem: (key: string) => store.get(key) ?? null,
    setItem: (key: string, value: string) => void store.set(key, value),
  })
  return { store, root }
}

/** A fresh copy of the module, which reads the storage when it loads. */
async function load() {
  vi.resetModules()
  return import('./useTheme')
}

describe('useTheme', () => {
  beforeEach(() => vi.unstubAllGlobals())
  afterEach(() => vi.unstubAllGlobals())

  it('starts on Dark when nothing is stored', async () => {
    const { root } = stubPage()
    const { theme } = await load()
    expect(theme.value).toBe('dark')
    expect(root.dataset.theme).toBe('dark')
    expect(root.classList.contains('dark')).toBe(true)
  })

  it('remembers the choice, applies it at once and reads it back after a restart', async () => {
    const { store, root } = stubPage()
    const first = await load()
    first.setTheme('midnight')
    expect(first.theme.value).toBe('midnight')
    expect(root.dataset.theme).toBe('midnight')
    expect(store.get('rain.theme')).toBe('midnight')

    const second = await load()
    expect(second.theme.value).toBe('midnight')
  })

  it('drops the dark class for Light only', async () => {
    const { root } = stubPage()
    const { setTheme } = await load()
    setTheme('light')
    expect(root.classList.contains('dark')).toBe(false)
    setTheme('dim')
    expect(root.classList.contains('dark')).toBe(true)
  })

  it('falls back to Dark for a stored value it does not know', async () => {
    const { root } = stubPage({ 'rain.theme': 'solarized' })
    const { theme } = await load()
    expect(theme.value).toBe('dark')
    expect(root.dataset.theme).toBe('dark')
  })

  it('ignores an unknown id in setTheme', async () => {
    const { store } = stubPage()
    const { setTheme, theme } = await load()
    setTheme('nope' as never)
    expect(theme.value).toBe('dark')
    expect(store.has('rain.theme')).toBe(false)
  })

  it('still works when the storage throws', async () => {
    stubPage()
    const blocked = () => {
      throw new Error('blocked')
    }
    vi.stubGlobal('localStorage', { getItem: blocked, setItem: blocked })
    const { setTheme, theme } = await load()
    expect(theme.value).toBe('dark')
    setTheme('dim')
    expect(theme.value).toBe('dim')
  })

  it('lists the five themes with their swatch colours', async () => {
    stubPage()
    const { themes } = await load()
    expect(themes.map((entry) => entry.id)).toEqual(['dark', 'high-contrast', 'dim', 'midnight', 'light'])
    for (const entry of themes) {
      expect(entry.name).not.toBe('')
      for (const colour of Object.values(entry.swatch)) expect(colour).toMatch(/^#[0-9a-f]{6}$/)
    }
  })
})
