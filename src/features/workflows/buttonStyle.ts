// The shadcn Button presses down with a small transform. Spec 5.5 allows that only for
// the clearly primary buttons of a pane, so the other pane buttons switch it off.
export const NO_PRESS = 'active:not-aria-[haspopup]:translate-y-0'
