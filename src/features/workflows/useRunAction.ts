// Lets a button in a detail pane run the same registry action as its chord, on the selected item.
import { runAction } from '@/actions/context'
import { useScreenState } from '@/composables/useScreen'

/** Returns a function that runs the selected item's action with this id ('log', 'run-now', ...). */
export function useRunAction(): (actionId: string) => void {
  const { actions } = useScreenState()
  return (actionId) => {
    const action = actions.value.find((candidate) => candidate.id === actionId)
    if (action) void runAction(action)
  }
}
