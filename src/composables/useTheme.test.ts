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

// ---- themes from VS Code ----

/** A root element with a style that remembers custom properties, as the page has. */
function stubStyledPage(stored: Record<string, string> = {}) {
  const { store, root } = stubPage(stored)
  const props = new Map<string, string>()
  const style = {
    setProperty: (name: string, value: string) => void props.set(name, value),
    removeProperty: (name: string) => void props.delete(name),
    item: (at: number) => [...props.keys()][at] ?? '',
    get length() {
      return props.size
    },
  }
  Object.assign(root, { style })
  return { store, root, props }
}

/** A made-up theme in the shape of the backend's answer. */
function madeUp(id: string, background = '#102030', dark = true) {
  return {
    id,
    label: id.split(':')[1] ?? id,
    dark,
    tokens: { background, foreground: '#f0f0f0', primary: '#d04080' } as Record<string, string>,
  }
}

function fakeApi(current: { id?: string; label?: string }) {
  const calls: string[] = []
  const api = {
    getVsCodeCurrentTheme: async () => {
      calls.push('current')
      return current
    },
    getVsCodeTheme: async (id: string) => {
      calls.push(`theme ${id}`)
      return madeUp(id, id.endsWith('Night') ? '#102030' : '#fafafa', id.endsWith('Night'))
    },
  }
  vi.doMock('@/data', () => ({ api }))
  return { calls, current }
}

describe('useTheme with a VS Code theme', () => {
  beforeEach(() => vi.unstubAllGlobals())
  afterEach(() => {
    vi.unstubAllGlobals()
    vi.doUnmock('@/data')
  })

  it('sets the tokens as custom properties, remembers them and brings them back at the next start', async () => {
    const { store, root, props } = stubStyledPage()
    fakeApi({})
    const first = await load()
    await first.chooseVsCodeTheme('a.b:Night')
    expect(first.theme.value).toBe('vscode')
    expect(root.dataset.theme).toBe('vscode')
    expect(props.get('--background')).toBe('#102030')
    expect(props.get('color-scheme')).toBe('dark')
    expect(store.get('rain.theme')).toBe('vscode')
    expect(JSON.parse(store.get('rain.theme.vscode')!).id).toBe('a.b:Night')

    props.clear()
    const second = await load()
    expect(second.theme.value).toBe('vscode')
    expect(props.get('--primary')).toBe('#d04080')
    expect(second.vscodeTheme.value?.label).toBe('Night')
  })

  it('removes the custom properties when a built-in theme is chosen again, and stops following', async () => {
    const { store, props } = stubStyledPage()
    fakeApi({ id: 'a.b:Night' })
    const { chooseVsCodeTheme, setTheme, followVsCode, setFollowVsCode, theme } = await load()
    await setFollowVsCode(true)
    expect(followVsCode.value).toBe(true)
    expect(theme.value).toBe('vscode')
    await chooseVsCodeTheme('a.b:Night')
    expect(followVsCode.value).toBe(false)
    setTheme('light')
    expect(props.size).toBe(0)
    expect(theme.value).toBe('light')
    expect(store.get('rain.theme')).toBe('light')
  })

  it('follows the theme VS Code uses, and asks for the theme file only when it changed', async () => {
    stubStyledPage()
    const api = fakeApi({ id: 'a.b:Night' })
    const { setFollowVsCode, checkFollowVsCode, vscodeTheme } = await load()
    await setFollowVsCode(true)
    expect(vscodeTheme.value?.id).toBe('a.b:Night')
    await checkFollowVsCode()
    expect(api.calls).toEqual(['current', 'theme a.b:Night', 'current'])
    api.current.id = 'a.b:Day'
    await checkFollowVsCode()
    expect(vscodeTheme.value?.id).toBe('a.b:Day')
    expect(vscodeTheme.value?.dark).toBe(false)
  })

  it('does not look at VS Code while the switch is off', async () => {
    stubStyledPage()
    const api = fakeApi({ id: 'a.b:Night' })
    const { checkFollowVsCode } = await load()
    await checkFollowVsCode()
    expect(api.calls).toEqual([])
  })

  it('looks once at the start when the window follows VS Code', async () => {
    stubStyledPage({ 'rain.theme.follow': '1' })
    const api = fakeApi({ id: 'a.b:Night' })
    const { vscodeTheme } = await load()
    await vi.waitFor(() => expect(vscodeTheme.value?.id).toBe('a.b:Night'))
    expect(api.calls).toEqual(['current', 'theme a.b:Night'])
  })

  it('keeps the theme and says why when VS Code uses a theme that is not installed', async () => {
    stubStyledPage()
    fakeApi({ label: 'Default Dark Modern' })
    const { setFollowVsCode, followVsCodeProblem, theme } = await load()
    await setFollowVsCode(true)
    expect(theme.value).toBe('dark')
    expect(followVsCodeProblem.value).toContain('Default Dark Modern')
  })

  it('ignores damaged stored colours and unsafe values', async () => {
    const { root } = stubStyledPage({
      'rain.theme': 'vscode',
      'rain.theme.vscode': '{"id":"a:b","label":"b","dark":true,"tokens":{"x":1}}',
    })
    const { theme, cleanTokens } = await load()
    expect(theme.value).toBe('dark')
    expect(root.dataset.theme).toBe('dark')
    expect(cleanTokens({ ok: '#fff', 'Bad Name': '#fff', evil: 'red; } body {', num: 4 })).toEqual({ ok: '#fff' })
  })
})

describe('the script of index.html', () => {
  it('applies a stored VS Code theme before the first paint, and a built-in one otherwise', async () => {
    const { readFileSync } = await import('node:fs')
    const script = /<script>([\s\S]*?)<\/script>/.exec(readFileSync('index.html', 'utf8'))?.[1] ?? ''
    const run = (stored: Record<string, string>) => {
      const props = new Map<string, string>()
      const classes = new Set<string>()
      const root = {
        dataset: {} as Record<string, string>,
        style: { setProperty: (name: string, value: string) => void props.set(name, value) },
        classList: { toggle: (name: string, on: boolean) => (on ? classes.add(name) : classes.delete(name)) },
      }
      new Function('localStorage', 'document', script)({ getItem: (key: string) => stored[key] ?? null }, { documentElement: root })
      return { root, props, classes }
    }
    const saved = JSON.stringify(madeUp('a.b:Day', '#fafafa', false))
    const vscode = run({ 'rain.theme': 'vscode', 'rain.theme.vscode': saved })
    expect(vscode.root.dataset.theme).toBe('vscode')
    expect(vscode.props.get('--background')).toBe('#fafafa')
    expect(vscode.props.get('color-scheme')).toBe('light')
    expect(vscode.classes.has('dark')).toBe(false)

    const lost = run({ 'rain.theme': 'vscode' })
    expect(lost.root.dataset.theme).toBe('dark')
    expect(lost.props.size).toBe(0)
    expect(run({ 'rain.theme': 'dim' }).root.dataset.theme).toBe('dim')
  })
})
