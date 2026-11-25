import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

// The commands load the zoom, which loads the shell and needs a window. A small stand-in is enough here.
vi.mock('@/shell', () => ({
  shell: {
    kind: 'tauri',
    bridge: { request: async () => undefined, openStream: () => () => {}, mediaUrl: (ref: string) => ref },
    hidden: { value: false },
    maximized: { value: false },
    workArea: async () => null,
    applyZoom: async () => {},
  },
}))

import type { ActionContext } from './types'

/** A stand-in for the page: a root element and a storage, as in useTheme.test.ts. */
function stubPage() {
  const store = new Map<string, string>()
  const root = {
    dataset: {} as Record<string, string>,
    classList: { toggle: () => true, contains: () => true },
  }
  vi.stubGlobal('document', { documentElement: root, createElement: () => ({}) })
  vi.stubGlobal('localStorage', {
    getItem: (key: string) => store.get(key) ?? null,
    setItem: (key: string, value: string) => void store.set(key, value),
  })
  return { store, root }
}

async function load() {
  vi.resetModules()
  const [commands, themeModule] = await Promise.all([import('./commands'), import('@/composables/useTheme')])
  return { commands: commands.commands, ...themeModule }
}

describe('theme commands', () => {
  beforeEach(() => vi.unstubAllGlobals())
  afterEach(() => vi.unstubAllGlobals())

  it('has one command for each theme', async () => {
    stubPage()
    const { commands, themes } = await load()
    const found = commands.filter((command) => command.id.startsWith('theme-'))
    expect(found.map((command) => command.id)).toEqual(themes.map((entry) => `theme-${entry.id}`))
    expect(found.map((command) => command.title)).toEqual(themes.map((entry) => `Theme: ${entry.name}`))
  })

  it('switches the theme and remembers it', async () => {
    const { store, root } = stubPage()
    const { commands, theme } = await load()
    await commands.find((command) => command.id === 'theme-light')!.run({} as ActionContext)
    expect(theme.value).toBe('light')
    expect(store.get('rain.theme')).toBe('light')
    expect(root.dataset.theme).toBe('light')
    await commands.find((command) => command.id === 'theme-dark')!.run({} as ActionContext)
    expect(theme.value).toBe('dark')
  })

  it('has unique ids across all commands', async () => {
    stubPage()
    const { commands } = await load()
    const ids = commands.map((command) => command.id)
    expect(new Set(ids).size).toBe(ids.length)
  })
})

describe('VS Code theme commands', () => {
  beforeEach(() => vi.unstubAllGlobals())
  afterEach(() => vi.unstubAllGlobals())

  it('has "Theme: follow VS Code" and keeps its id out of the built-in theme ids', async () => {
    stubPage()
    const { commands } = await load()
    const follow = commands.find((command) => command.title === 'Theme: follow VS Code')
    expect(follow?.id).toBe('vscode-theme-follow')
    expect(commands.filter((command) => command.id.startsWith('theme-')).every((c) => !c.id.includes('vscode'))).toBe(true)
    expect(commands.some((command) => command.title === 'Theme: stop following VS Code')).toBe(true)
  })
})
