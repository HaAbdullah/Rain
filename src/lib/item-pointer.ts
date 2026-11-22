// The mouse rules for selectable items (spec 3.8), in one place for ItemList and ItemGrid.

export interface ItemPointerOptions<T> {
  selectedId: () => string | null
  /** True for a list without a detail pane (Home): one click runs the primary action. */
  clickRuns: () => boolean
  select: (id: string) => void
  activate: (item: T) => void
  context: (item: T, event: MouseEvent) => void
}

export function useItemPointer<T extends { id: string }>(options: ItemPointerOptions<T>) {
  function select(item: T): void {
    if (item.id !== options.selectedId()) options.select(item.id)
  }

  return {
    /** Click selects. Where a click runs, it also runs the primary action. */
    onClick(item: T, event: MouseEvent): void {
      select(item)
      // `detail` counts the clicks: the second click of a double-click must not run the action again.
      if (options.clickRuns() && event.detail <= 1) options.activate(item)
    },

    /** Double-click runs the primary action where a single click only selects. */
    onDoubleClick(item: T): void {
      if (!options.clickRuns()) options.activate(item)
    },

    /** Right-click selects and asks for the action panel at the pointer. */
    onContextMenu(item: T, event: MouseEvent): void {
      event.preventDefault()
      select(item)
      options.context(item, event)
    },
  }
}
