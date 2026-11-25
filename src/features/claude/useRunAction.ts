// Lets a button in the detail pane run the same registry action as its key, on the selected file.
import { runAction } from '@/actions/context'
import { useScreenState } from '@/composables/useScreen'

/** Returns a function that runs the selected item's action with this id ('open', 'reveal', 'copy'). */
export function useRunAction(): (actionId: string) => void {
  const { actions } = useScreenState()
  return (actionId) => {
    const action = actions.value.find((candidate) => candidate.id === actionId)
    if (action) void runAction(action)
  }
}
