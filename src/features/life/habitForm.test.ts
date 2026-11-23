import { describe, expect, it } from 'vitest'
import { lucideIcon } from '@/lib/icon-map'
import { HABIT_ICONS, clampTimes, nameFromProperty, propertyFromName, timesMax } from './habitForm'

describe('propertyFromName', () => {
  it('makes a lower case name with dashes', () => {
    expect(propertyFromName('Gym')).toBe('gym')
    expect(propertyFromName('  Morning run ')).toBe('morning-run')
    expect(propertyFromName('Read_more  books')).toBe('read-more-books')
  })

  it('drops characters a property name cannot hold', () => {
    expect(propertyFromName('Walk: 10k!')).toBe('walk-10k')
    expect(propertyFromName('#tags [x]')).toBe('tags-x')
    expect(propertyFromName('---')).toBe('')
    expect(propertyFromName('')).toBe('')
  })
})

describe('nameFromProperty', () => {
  it('turns dashes into spaces and capitalises', () => {
    expect(nameFromProperty('morning-run')).toBe('Morning run')
    expect(nameFromProperty('gym')).toBe('Gym')
    expect(nameFromProperty('')).toBe('')
  })
})

describe('times', () => {
  it('has the maximum of the cadence', () => {
    expect(timesMax('daily')).toBe(1)
    expect(timesMax('weekly')).toBe(7)
    expect(timesMax('monthly')).toBe(31)
  })

  it('holds a typed value to 1 and the maximum', () => {
    expect(clampTimes('weekly', 0)).toBe(1)
    expect(clampTimes('weekly', 9)).toBe(7)
    expect(clampTimes('weekly', 3.4)).toBe(3)
    expect(clampTimes('monthly', 40)).toBe(31)
    expect(clampTimes('daily', 5)).toBe(1)
    expect(clampTimes('weekly', Number.NaN)).toBe(1)
  })
})

describe('the icon grid', () => {
  it('offers 28 icons, each one known to the icon map', () => {
    expect(HABIT_ICONS).toHaveLength(28)
    expect(new Set(HABIT_ICONS).size).toBe(28)
    // An unknown name gives the dashed-square fallback, so a known one must differ from it.
    const fallback = lucideIcon('no-such-icon')
    for (const name of HABIT_ICONS) expect(lucideIcon(name), name).not.toBe(fallback)
  })
})
