import { createSSRApp, h } from 'vue'
import { renderToString } from 'vue/server-renderer'
import { describe, expect, it } from 'vitest'
import type { Cadence, HabitSummary } from '@/data/types'
import AllHabitsHeat from './AllHabitsHeat.vue'
import { heatModel, heatStart, heatStep, weeksThatFit } from './allHabitsHeat'

function habit(id: string, cadence: Cadence = 'daily', todayDone = false): HabitSummary {
  return {
    habit: { id, name: id, icon: { kind: 'lucide', value: 'square-check' }, property: id, cadence, times: 1 },
    today: '2026-11-22',
    todayDone,
    periodCount: 0,
    periodTarget: 1,
    cells: [],
  }
}

// Sunday 22 November 2026.
const TODAY = '2026-11-22'

function find(model: ReturnType<typeof heatModel>, date: string) {
  const day = model.days.find((entry) => entry.date === date)
  if (!day) throw new Error(`no cell for ${date}`)
  return day
}

describe('heatStep', () => {
  it('maps a value to five steps', () => {
    expect([0, 0.01, 0.25, 0.26, 0.5, 0.51, 0.75, 0.76, 1].map(heatStep)).toEqual([0, 1, 1, 2, 2, 3, 3, 4, 4])
  })
})

describe('heatModel', () => {
  it('has no steps above 0 when there are no habits', () => {
    const model = heatModel([], [{ date: '2026-11-20', done: ['gone'] }], TODAY, 4)
    expect(model.days.every((day) => day.step === 0 && day.total === 0)).toBe(true)
    expect(model.fullDays).toBe(0)
  })

  it('divides the ticks by the daily habits', () => {
    const habits = [habit('a'), habit('b'), habit('c'), habit('d')]
    const model = heatModel(habits, [{ date: '2026-11-20', done: ['a', 'b'] }], TODAY, 4)
    expect(find(model, '2026-11-20')).toMatchObject({ done: 2, total: 4, value: 0.5, step: 2 })
    expect(find(model, '2026-11-19')).toMatchObject({ done: 0, step: 0 })
  })

  it('caps a day with more ticks than daily habits at 1', () => {
    const habits = [habit('a'), habit('w', 'weekly')]
    const model = heatModel(habits, [{ date: '2026-11-20', done: ['a', 'w'] }], TODAY, 4)
    expect(find(model, '2026-11-20')).toMatchObject({ done: 1, total: 1, value: 1, step: 4 })
    expect(model.fullDays).toBe(1)
  })

  it('counts ticks of weekly habits and divides by the daily ones', () => {
    const habits = [habit('a'), habit('b'), habit('w', 'weekly')]
    const model = heatModel(habits, [{ date: '2026-11-20', done: ['w'] }], TODAY, 4)
    expect(find(model, '2026-11-20')).toMatchObject({ done: 1, total: 2, step: 2 })
  })

  it('divides by all habits when none is daily, and ignores deleted habits', () => {
    const habits = [habit('w1', 'weekly'), habit('m1', 'monthly')]
    const model = heatModel(habits, [{ date: '2026-11-20', done: ['w1', 'gone'] }], TODAY, 4)
    expect(find(model, '2026-11-20')).toMatchObject({ done: 1, total: 2, value: 0.5 })
  })

  it('takes today from the habit list, so a tick shows at once', () => {
    const habits = [habit('a', 'daily', true), habit('b')]
    const model = heatModel(habits, [], TODAY, 4)
    expect(find(model, TODAY)).toMatchObject({ done: 1, total: 2, step: 2 })
  })

  it('starts on a Monday, ends today and draws no future day', () => {
    const model = heatModel([habit('a')], [], '2026-11-18', 3) // a Wednesday
    expect(model.weekCount).toBe(3)
    expect(model.days[0]).toMatchObject({ date: '2026-11-02', column: 0, row: 0 })
    expect(model.days[model.days.length - 1]).toMatchObject({ date: '2026-11-18', column: 2, row: 2 })
    expect(model.days).toHaveLength(17)
  })

  it('keeps the first and last day of the range inside the grid', () => {
    expect(heatStart(TODAY, 53)).toBe('2025-11-17')
    const model = heatModel([habit('a')], [], TODAY, 53)
    expect(model.days[0].date).toBe('2025-11-17')
    expect(model.days[model.days.length - 1]).toMatchObject({ date: TODAY, column: 52, row: 6 })
    // More weeks than a year are not drawn.
    expect(heatModel([habit('a')], [], TODAY, 80).weekCount).toBe(53)
  })

  it('lines the weeks up across a year change', () => {
    // 1 January 2026 is a Thursday: it sits in the week that began on Monday 29 December.
    const model = heatModel([habit('a')], [{ date: '2026-01-01', done: ['a'] }], '2026-01-10', 3)
    expect(model.days[0]).toMatchObject({ date: '2025-12-22', column: 0, row: 0 })
    expect(find(model, '2025-12-31')).toMatchObject({ column: 1, row: 2 })
    expect(find(model, '2026-01-01')).toMatchObject({ column: 1, row: 3, step: 4 })
    expect(find(model, '2026-01-05')).toMatchObject({ column: 2, row: 0 })
  })

  it('names each month once, over its first column', () => {
    const model = heatModel([habit('a')], [], TODAY, 53)
    const names = model.labels.map((label) => label.name)
    expect(new Set(names).size).toBe(names.length)
    expect(names[names.length - 1]).toBe('Nov')
    expect(model.labels.every((label, i, all) => i === 0 || label.column - all[i - 1].column >= 3)).toBe(true)
  })
})

describe('weeksThatFit', () => {
  it('fits as many weeks as the width allows, from 1 to 53', () => {
    expect(weeksThatFit(686, 10, 3)).toBe(53)
    expect(weeksThatFit(685, 10, 3)).toBe(52)
    expect(weeksThatFit(5000, 10, 3)).toBe(53)
    expect(weeksThatFit(3, 10, 3)).toBe(1)
  })
})

describe('AllHabitsHeat', () => {
  async function render(habits: HabitSummary[], days: { date: string; done: string[] }[]) {
    const app = createSSRApp({ render: () => h(AllHabitsHeat, { habits, days, today: TODAY }) })
    return renderToString(app)
  }

  it('draws one cell per day with its step as a heat token, and marks today', async () => {
    const habits = [habit('a'), habit('b'), habit('c'), habit('d')]
    const html = await render(habits, [
      { date: '2026-11-21', done: ['a'] },
      { date: '2026-11-20', done: ['a', 'b'] },
      { date: '2026-11-19', done: ['a', 'b', 'c'] },
      { date: '2026-11-18', done: ['a', 'b', 'c', 'd'] },
    ])
    for (const step of [0, 1, 2, 3, 4]) {
      expect(html).toContain(`data-step="${step}"`)
      expect(html).toContain(`bg-heat-${step}`)
    }
    expect(html).toContain('title="Fri 20 Nov: 2 of 4 done"')
    expect(html.match(/outline-foreground/g)).toHaveLength(1)
    expect(html).toContain('All habits')
    expect(html).toContain('1 day with everything done')
    expect(html).not.toMatch(/cursor-pointer/)
    expect(html).not.toMatch(/#[0-9a-f]{3,6}\b/i)
  })
})
