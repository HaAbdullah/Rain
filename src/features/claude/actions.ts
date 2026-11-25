// Actions for a file of the Claude library (Enter, Ctrl+Enter, Ctrl+K). The library is read only:
// nothing here changes a file. Open hands the file to the editor that Projects uses.
import { copyAction, hideAfterSuccess, revealAction } from '@/actions/common'
import type { ActionProvider, TargetOf } from '@/actions/types'

export const claudeItemActions: ActionProvider<TargetOf<'claudeItem'>> = ({ item }) => [
  {
    id: 'open',
    label: 'Open in editor',
    icon: 'code-xml',
    slot: 'primary',
    async run(ctx) {
      await ctx.api.openClaudeLibraryItem(item.id)
      await hideAfterSuccess(ctx)
    },
  },
  revealAction(item.path, 'secondary'),
  copyAction('path', item.path),
]
