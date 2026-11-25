import { describe, expect, it, vi } from 'vitest'

// The keymap loads the shell, which needs a window. A small stand-in is enough here.
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

import { canonicalCombo, combosOf, keymap } from './keymap'

function binding(id: string) {
  const found = keymap.find((candidate) => candidate.id === id)
  if (!found) throw new Error(`no binding ${id}`)
  return found
}

describe('keymap: window and zoom keys', () => {
  it('F11 maximises, it does not go full screen', () => {
    expect(combosOf(binding('maximize'))).toEqual(['F11'])
    expect(binding('maximize').label).toBe('Maximise / restore')
    expect(keymap.some((entry) => entry.id === 'fullscreen')).toBe(false)
  })

  it('Ctrl with plus zooms in, on the main block and the number pad', () => {
    expect(combosOf(binding('zoom-in'))).toEqual(expect.arrayContaining(['Ctrl+Equal', 'Ctrl+NumpadAdd']))
  })

  it('Ctrl with minus zooms out, on the main block and the number pad', () => {
    expect(combosOf(binding('zoom-out'))).toEqual(['Ctrl+Minus', 'Ctrl+NumpadSubtract'])
  })

  it('Ctrl+0 resets', () => {
    expect(combosOf(binding('zoom-reset'))).toContain('Ctrl+Digit0')
  })

  it('no chord belongs to two bindings that are always live', () => {
    const seen = new Map<string, string>()
    for (const entry of keymap.filter((candidate) => candidate.when === undefined)) {
      for (const combo of combosOf(entry)) {
        const key = canonicalCombo(combo)
        expect(seen.get(key), `${key} is bound twice`).toBeUndefined()
        seen.set(key, entry.id)
      }
    }
  })
})

describe('keymap: back and forward', () => {
  it('are actions that the mouse buttons, Alt with an arrow and the browser keys all send', () => {
    expect(combosOf(binding('history-back'))).toEqual(['Alt+ArrowLeft', 'BrowserBack', 'MouseBack'])
    expect(combosOf(binding('history-forward'))).toEqual(['Alt+ArrowRight', 'BrowserForward', 'MouseForward'])
  })
})
