// What each tab view exposes to WorkflowsScreen, which passes it on to the shell's
// ScreenController (the screen calls useScreen once; the tabs come and go).
import type { ActionTarget } from '@/actions/types'
import type { Dir } from '@/composables/useScreen'

export interface ViewController {
  target: ActionTarget | null
  move(dir: Dir): boolean
  page(dir: -1 | 1): void
  group(dir: -1 | 1): void
}
