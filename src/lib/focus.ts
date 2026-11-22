// Focus rests in the search field; lists, grids and strips show selection as
// state and must not take DOM focus when they are clicked (spec 1.6).

/**
 * `mousedown` handler that stops a click from moving focus out of the search
 * field. The click itself still fires. Text fields inside the area keep the
 * normal behaviour, because a Settings field has to take focus to be edited.
 */
export function keepFocus(event: MouseEvent): void {
  const target = event.target
  if (target instanceof Element && target.closest('input, textarea, select, [contenteditable="true"]')) return
  event.preventDefault()
}
