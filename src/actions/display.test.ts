import { describe, expect, it } from 'vitest'
import { keysOf, orderActions, targetName } from './display'
import type { ActionTarget, RainAction } from './types'

const make = (id: string, extra: Partial<RainAction> = {}): RainAction => ({
  id,
  label: id,
  icon: 'x',
  slot: 'more',
  run: () => {},
  ...extra,
})

describe('keysOf', () => {
  it('maps the slots to Enter and Ctrl+Enter', () => {
    expect(keysOf({ slot: 'primary' })).toBe('Enter')
    expect(keysOf({ slot: 'secondary' })).toBe('Ctrl+Enter')
  })
  it('uses the chord for a more action, or nothing', () => {
    expect(keysOf({ slot: 'more', keys: 'Ctrl+KeyY' })).toBe('Ctrl+KeyY')
    expect(keysOf({ slot: 'more' })).toBeUndefined()
  })
})

describe('orderActions', () => {
  it('puts danger last and keeps the rest in order', () => {
    const list = [make('a', { danger: true }), make('b'), make('c')]
    expect(orderActions(list).map((a) => a.id)).toEqual(['b', 'c', 'a'])
  })
})

describe('targetName', () => {
  it('reads the name, then falls back to the type', () => {
    const named = { type: 'app', id: '1', item: { name: 'Code' } } as unknown as ActionTarget
    const bare = { type: 'pool', id: '2', item: {} } as unknown as ActionTarget
    expect(targetName(named)).toBe('Code')
    expect(targetName(bare)).toBe('pool')
  })
})
