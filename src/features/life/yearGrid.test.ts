import { describe, expect, it } from 'vitest'
import { yearGrid } from './yearGrid'

describe('yearGrid', () => {
  it('puts 1 January 2026, a Thursday, in column 0 on row 3', () => {
    const { cells } = yearGrid(2026, '2026-11-22')
    expect(cells[0]).toEqual({ date: '2026-01-01', column: 0, row: 3 })
    expect(cells[3]).toEqual({ date: '2026-01-04', column: 0, row: 6 }) // the first Sunday ends week 0
    expect(cells[4]).toEqual({ date: '2026-01-05', column: 1, row: 0 }) // the first Monday starts week 1
  })

  it('puts Sunday 22 November 2026 in column 46 on row 6', () => {
    const { cells } = yearGrid(2026, '2026-11-22')
    expect(cells.find((cell) => cell.date === '2026-11-22')).toEqual({
      date: '2026-11-22',
      column: 46,
      row: 6,
    })
  })

  it('does not draw days after today', () => {
    const { cells } = yearGrid(2026, '2026-11-22')
    expect(cells).toHaveLength(326)
    expect(cells.at(-1)?.date).toBe('2026-11-22')
    expect(yearGrid(2027, '2026-11-22').cells).toEqual([])
  })

  it('draws a whole past year, with 53 weeks when it needs them', () => {
    const grid = yearGrid(2026, '2027-03-01')
    expect(grid.cells).toHaveLength(365)
    expect(grid.weekCount).toBe(53)
  })

  it('labels a month at the column of its first day, once that day has come', () => {
    const { labels } = yearGrid(2026, '2026-11-22')
    expect(labels.map((label) => label.name)).toEqual([
      'Jan', 'Feb', 'Mar', 'Apr', 'May', 'Jun', 'Jul', 'Aug', 'Sep', 'Oct', 'Nov',
    ])
    expect(labels[0]).toEqual({ name: 'Jan', column: 0 })
    expect(labels[10].column).toBe(43) // 1 November 2026 is a Sunday: day 304 + 3 = 307, week 43
    expect(yearGrid(2026, '2026-02-10').labels.map((label) => label.name)).toEqual(['Jan', 'Feb'])
  })

  it('handles a leap year', () => {
    const { cells, weekCount } = yearGrid(2024, '2024-12-31')
    expect(cells).toHaveLength(366)
    expect(cells.some((cell) => cell.date === '2024-02-29')).toBe(true)
    // 1 January 2024 is a Monday, so the lead is 0 and 366 days need 53 columns.
    expect(cells[0]).toEqual({ date: '2024-01-01', column: 0, row: 0 })
    expect(weekCount).toBe(53)
  })
})
