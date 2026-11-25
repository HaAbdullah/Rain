import { describe, expect, it, vi } from 'vitest'

vi.mock('@/shell', () => ({ shell: { kind: 'tauri' } }))

import { useNav } from './useNav'

describe('useNav: back and forward', () => {
  it('steps back through the screens visited and forward again', () => {
    const nav = useNav()
    nav.go('apps')
    nav.go('games')
    expect(nav.stepBack()).toBe(true)
    expect(nav.route.value.name).toBe('apps')
    expect(nav.stepForward()).toBe(true)
    expect(nav.route.value.name).toBe('games')
    expect(nav.stepForward()).toBe(false)
  })

  it('a new screen after a step back leaves nothing ahead', () => {
    const nav = useNav()
    nav.go('apps')
    nav.go('games')
    nav.stepBack()
    nav.go('life')
    expect(nav.stepForward()).toBe(false)
    expect(nav.route.value.name).toBe('life')
  })

  it('does nothing at the start of the history', () => {
    const nav = useNav()
    while (nav.stepBack()) {
      // back to the first screen
    }
    const first = nav.route.value.name
    expect(nav.stepBack()).toBe(false)
    expect(nav.route.value.name).toBe(first)
  })
})
