import { describe, expect, it } from 'vitest'
import {
  dropSlot,
  moveBy,
  moveToSlot,
  reconcileOrder,
  sameOrder,
  sectionCombo,
  slotChangesOrder,
} from './rail-order'

const ids = ['home', 'apps', 'projects', 'games'] as const

describe('reconcileOrder', () => {
  it('keeps a stored order that fits', () => {
    expect(reconcileOrder(['games', 'home', 'projects', 'apps'], ids)).toEqual(['games', 'home', 'projects', 'apps'])
  })

  it('drops unknown and repeated ids', () => {
    expect(reconcileOrder(['games', 'gone', 'games', 'home'], ids)).toEqual(['games', 'home', 'apps', 'projects'])
  })

  it('appends sections that are new, in their default order', () => {
    expect(reconcileOrder(['projects', 'home'], ids)).toEqual(['projects', 'home', 'apps', 'games'])
    expect(reconcileOrder(['projects', 'home'], [...ids, 'claude'])).toEqual([
      'projects',
      'home',
      'apps',
      'games',
      'claude',
    ])
  })

  it('gives the default order for anything that is not a list of text', () => {
    for (const bad of [null, undefined, 'home', 7, {}, [1, null]]) {
      expect(reconcileOrder(bad, ids)).toEqual([...ids])
    }
  })
})

describe('moveBy', () => {
  it('swaps with the neighbour', () => {
    expect(moveBy(ids, 'apps', -1)).toEqual(['apps', 'home', 'projects', 'games'])
    expect(moveBy(ids, 'apps', 1)).toEqual(['home', 'projects', 'apps', 'games'])
  })

  it('stays at the ends and for an unknown id', () => {
    expect(moveBy(ids, 'home', -1)).toEqual([...ids])
    expect(moveBy(ids, 'games', 1)).toEqual([...ids])
    expect(moveBy(ids, 'nope', 1)).toEqual([...ids])
  })
})

describe('moveToSlot', () => {
  it('moves an icon down into a gap', () => {
    expect(moveToSlot(ids, 'home', 3)).toEqual(['apps', 'projects', 'home', 'games'])
    expect(moveToSlot(ids, 'home', 4)).toEqual(['apps', 'projects', 'games', 'home'])
  })

  it('moves an icon up into a gap', () => {
    expect(moveToSlot(ids, 'games', 0)).toEqual(['games', 'home', 'apps', 'projects'])
    expect(moveToSlot(ids, 'games', 1)).toEqual(['home', 'games', 'apps', 'projects'])
  })

  it('leaves the order when the gap is next to the icon itself', () => {
    expect(moveToSlot(ids, 'apps', 1)).toEqual([...ids])
    expect(moveToSlot(ids, 'apps', 2)).toEqual([...ids])
    expect(slotChangesOrder(ids, 'apps', 1)).toBe(false)
    expect(slotChangesOrder(ids, 'apps', 2)).toBe(false)
    expect(slotChangesOrder(ids, 'apps', 3)).toBe(true)
  })

  it('keeps a slot outside the list inside it', () => {
    expect(moveToSlot(ids, 'apps', -5)).toEqual(['apps', 'home', 'projects', 'games'])
    expect(moveToSlot(ids, 'apps', 99)).toEqual(['home', 'projects', 'games', 'apps'])
  })
})

describe('dropSlot', () => {
  it('counts the icons above the pointer', () => {
    const centres = [20, 60, 100]
    expect(dropSlot(centres, 5)).toBe(0)
    expect(dropSlot(centres, 61)).toBe(2)
    expect(dropSlot(centres, 500)).toBe(3)
  })
})

describe('sectionCombo', () => {
  it('numbers the places from the top, up to nine', () => {
    expect(sectionCombo(0)).toBe('Ctrl+Digit1')
    expect(sectionCombo(8)).toBe('Ctrl+Digit9')
    expect(sectionCombo(9)).toBeUndefined()
    expect(sectionCombo(-1)).toBeUndefined()
  })
})

describe('sameOrder', () => {
  it('compares position by position', () => {
    expect(sameOrder(['a', 'b'], ['a', 'b'])).toBe(true)
    expect(sameOrder(['a', 'b'], ['b', 'a'])).toBe(false)
    expect(sameOrder(['a'], ['a', 'b'])).toBe(false)
  })
})
