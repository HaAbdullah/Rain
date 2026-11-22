// Keyboard selection for lists and grids (spec 6.2). Selection is state, not DOM
// focus: the caret stays in the search field and this composable decides which
// item is "the selected one". ItemList and ItemGrid only draw the result.
//
// The model: groups of items, each group laid out in rows of N columns
// (N = 1 for a list). Up/Down walk rows and cross into the next group keeping
// the column; Left/Right walk inside a row.
import { computed, reactive, toRef, watch, type ComputedRef, type Ref } from 'vue'
import type { Dir } from './useScreen'
import { useViewState } from './useViewState'

export interface SelGroup<T> {
  id: string
  title?: string
  count?: number
  items: T[]
  /** Column count of this group when it differs from `opts.columns` (Projects: 3 above 5). */
  columns?: number
}

export interface SelectionOptions<T> {
  /** Live column count of the grid. Leave out for a list. */
  columns?: Ref<number>
  /** The item to select when nothing is selected yet. Default: the first item. */
  initial?: (all: T[]) => string | undefined
  /** Keeps the selected id in useViewState, so it survives leaving the screen. */
  persistKey?: string
  /**
   * A different list has begun (a new filter text): when this value changes, the
   * first item is selected (spec 3.5). Without it a vanished selection falls back
   * to the item nearest its old place, which is right for reloads only.
   */
  resetOn?: Ref<unknown>
}

export interface Selection<T> {
  selectedId: Ref<string | null>
  selected: ComputedRef<T | null>
  move(dir: Dir): boolean
  page(dir: -1 | 1): void
  group(dir: -1 | 1): void
  select(id: string): void
}

/** Rows moved by PageUp/PageDown. Grid rows are several times taller than list rows. */
const PAGE_ROWS_LIST = 8
const PAGE_ROWS_GRID = 3

interface Position<T> {
  item: T
  /** Index of the group in `groups`. */
  group: number
  /** Index of the item inside its group. */
  index: number
}

/** The column a vertical move tries to stay in, remembered across short rows. */
interface GoalColumn {
  /** The selection this goal belongs to; any other selection change discards it. */
  forId: string
  col: number
  cols: number
}

/**
 * Maps a column between groups of different widths by position, not by index:
 * the middle of 5 columns lands in the middle of 3.
 */
function mapColumn(col: number, from: number, to: number): number {
  if (from === to) return col
  return Math.min(to - 1, Math.floor(((col + 0.5) * to) / from))
}

export function useSelection<T extends { id: string }>(
  groups: Ref<SelGroup<T>[]>,
  opts: SelectionOptions<T> = {},
): Selection<T> {
  const stored = opts.persistKey
    ? useViewState(opts.persistKey, { id: null as string | null })
    : reactive({ id: null as string | null })
  const selectedId = toRef(stored, 'id')

  const positions = computed<Position<T>[]>(() =>
    groups.value.flatMap((group, groupIndex) =>
      group.items.map((item, index) => ({ item, group: groupIndex, index })),
    ),
  )

  const current = computed(
    () => positions.value.find((position) => position.item.id === selectedId.value) ?? null,
  )
  const selected = computed(() => current.value?.item ?? null)

  let goal: GoalColumn | null = null
  /** Where the selection last was in the flat order, for the nearest-item fallback. */
  let lastFlatIndex = -1

  function columnsOf(group: SelGroup<T>): number {
    return Math.max(1, group.columns ?? opts.columns?.value ?? 1)
  }

  /** Keeps the selection valid when the items change (filter, reload) or the state is reset. */
  function reconcile() {
    const all = positions.value
    // Nothing to choose from yet. A persisted id is kept until the data arrives.
    if (all.length === 0) return

    const index = all.findIndex((position) => position.item.id === selectedId.value)
    if (index >= 0) {
      lastFlatIndex = index
      return
    }

    if (selectedId.value !== null && lastFlatIndex >= 0) {
      // The selected item is gone: take the one that now sits nearest to its place.
      selectedId.value = all[Math.min(lastFlatIndex, all.length - 1)].item.id
      return
    }

    const wanted = opts.initial?.(all.map((position) => position.item))
    const known = all.some((position) => position.item.id === wanted)
    selectedId.value = known && wanted ? wanted : all[0].item.id
  }

  watch(
    [positions, selectedId],
    () => {
      if (selectedId.value === null) lastFlatIndex = -1
      reconcile()
    },
    { immediate: true },
  )

  if (opts.resetOn) {
    watch(opts.resetOn, () => {
      const first = positions.value[0]
      if (first) selectedId.value = first.item.id
    })
  }

  /** The remembered goal column if it still belongs to `from`, else a fresh one. */
  function goalFor(from: Position<T>): GoalColumn {
    if (goal?.forId === from.item.id) return goal
    const cols = columnsOf(groups.value[from.group])
    return { forId: from.item.id, col: from.index % cols, cols }
  }

  /** The item one row up or down from `from`, or null at the very top or bottom. */
  function verticalNeighbour(from: Position<T>, want: GoalColumn, step: -1 | 1): T | null {
    const group = groups.value[from.group]
    const cols = columnsOf(group)
    const rowCount = Math.ceil(group.items.length / cols)
    const nextRow = Math.floor(from.index / cols) + step
    if (nextRow >= 0 && nextRow < rowCount) {
      // A short last row has no item under every column: clamp to its last item.
      const index = nextRow * cols + mapColumn(want.col, want.cols, cols)
      return group.items[Math.min(index, group.items.length - 1)]
    }

    let next = from.group + step
    while (groups.value[next] && groups.value[next].items.length === 0) next += step
    const target = groups.value[next]
    if (!target) return null

    const targetCols = columnsOf(target)
    const targetRow = step === 1 ? 0 : Math.ceil(target.items.length / targetCols) - 1
    const index = targetRow * targetCols + mapColumn(want.col, want.cols, targetCols)
    return target.items[Math.min(index, target.items.length - 1)]
  }

  /** Moves one row. Returns false at the edge. */
  function stepVertical(step: -1 | 1): boolean {
    const from = current.value
    if (!from) return false
    const want = goalFor(from)
    const next = verticalNeighbour(from, want, step)
    if (!next) return false
    goal = { ...want, forId: next.id }
    selectedId.value = next.id
    return true
  }

  function stepHorizontal(step: -1 | 1): boolean {
    const from = current.value
    if (!from) return false
    const group = groups.value[from.group]
    const cols = columnsOf(group)
    // One column is a list: Left and Right belong to the caret.
    if (cols === 1) return false

    const index = from.index + step
    const sameRow = Math.floor(index / cols) === Math.floor(from.index / cols)
    if (sameRow && index >= 0 && index < group.items.length) selectedId.value = group.items[index].id
    // At the end of a row the key is still the grid's: the caret must not start moving.
    return true
  }

  function selectFirstIfNone(): boolean {
    if (current.value || positions.value.length === 0) return false
    reconcile()
    return true
  }

  function move(dir: Dir): boolean {
    if (positions.value.length === 0) return false
    if (selectFirstIfNone()) return true
    if (dir === 'up' || dir === 'down') {
      stepVertical(dir === 'up' ? -1 : 1)
      return true
    }
    return stepHorizontal(dir === 'left' ? -1 : 1)
  }

  function page(dir: -1 | 1): void {
    if (selectFirstIfNone() || !current.value) return
    const isGrid = columnsOf(groups.value[current.value.group]) > 1
    const rows = isGrid ? PAGE_ROWS_GRID : PAGE_ROWS_LIST
    for (let moved = 0; moved < rows; moved += 1) {
      if (!stepVertical(dir)) break
    }
  }

  /** Jumps to the first item of the previous or next group that has items. */
  function group(dir: -1 | 1): void {
    if (selectFirstIfNone() || !current.value) return
    let next = current.value.group + dir
    while (groups.value[next] && groups.value[next].items.length === 0) next += dir
    const target = groups.value[next] ?? (dir === -1 ? groups.value[current.value.group] : undefined)
    if (target) selectedId.value = target.items[0].id
  }

  function select(id: string): void {
    if (positions.value.some((position) => position.item.id === id)) selectedId.value = id
  }

  return { selectedId, selected, move, page, group, select }
}
