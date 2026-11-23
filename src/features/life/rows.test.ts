import { describe, expect, it } from 'vitest'
import type { Cadence, HabitSummary } from '@/data/types'
import { groupHabits } from './rows'

function summary(id: string, name: string, cadence: Cadence): HabitSummary {
  return {
    habit: { id, name, icon: { kind: 'lucide', value: 'square-check' }, property: id, cadence, times: 1 },
    today: '2026-11-22',
    todayDone: false,
    periodCount: 0,
    periodTarget: 1,
    cells: [],
  }
}

const LIST = [
  summary('gym', 'Gym', 'weekly'),
  summary('read', 'Read', 'daily'),
  summary('review', 'Review', 'monthly'),
  summary('walk', 'Walk the dog', 'daily'),
]

describe('groupHabits', () => {
  it('puts Daily, Weekly and Monthly in that order', () => {
    const groups = groupHabits(LIST, '')
    expect(groups.map((group) => group.id)).toEqual(['daily', 'weekly', 'monthly'])
    expect(groups.map((group) => group.title)).toEqual(['Daily', 'Weekly', 'Monthly'])
  })

  it('keeps the order of the list inside a group and counts the rows', () => {
    const [daily, weekly, monthly] = groupHabits(LIST, '')
    expect(daily.items.map((item) => item.id)).toEqual(['read', 'walk'])
    expect(daily.count).toBe(2)
    expect(weekly.items.map((item) => item.id)).toEqual(['gym'])
    expect(monthly.items.map((item) => item.id)).toEqual(['review'])
  })

  it('gives every row the id of its habit', () => {
    const [daily] = groupHabits(LIST, '')
    expect(daily.items[0].id).toBe('read')
    expect(daily.items[0].habit.name).toBe('Read')
  })

  it('keeps an empty group as a group with no items', () => {
    const groups = groupHabits([summary('read', 'Read', 'daily')], '')
    expect(groups).toHaveLength(3)
    expect(groups[1].items).toEqual([])
    expect(groups[1].count).toBe(0)
  })

  it('narrows the rows by habit name', () => {
    const groups = groupHabits(LIST, 'rev')
    expect(groups.flatMap((group) => group.items.map((item) => item.id))).toEqual(['review'])
    expect(groups[0].count).toBe(0)
    expect(groupHabits(LIST, 'zzzz').every((group) => group.items.length === 0)).toBe(true)
  })

  it('gives nothing for no habits', () => {
    expect(groupHabits([], '').every((group) => group.items.length === 0)).toBe(true)
  })
})
