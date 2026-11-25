// Actions for a tile of the Hardware screen (spec 7.7). Enter opens what the tile opens: the
// Disk screen, a program, or a web page. Ctrl+R reads the devices again.
import { copyAction, hideAfterSuccess } from '@/actions/common'
import type { ActionProvider, RainAction, TargetOf } from '@/actions/types'
import { refreshResources } from '@/composables/useResource'
import { openLabel } from './model'

/** The key of the one resource of the screen. The backend reads the PC only when it is fetched. */
export const HARDWARE_RESOURCE = 'hardware'

const refresh: RainAction = {
  id: 'refresh',
  label: 'Read devices again',
  icon: 'refresh-cw',
  slot: 'more',
  keys: 'Ctrl+KeyR',
  run: () => refreshResources(HARDWARE_RESOURCE),
}

export const hardwareActions: ActionProvider<TargetOf<'hardware'>> = ({ item: tile }) => {
  const more: RainAction[] = [refresh]
  if (tile.product) more.push(copyAction('name', tile.product))

  if (tile.opens.kind === 'disk') {
    return [
      {
        id: 'open-disk',
        label: openLabel(tile),
        icon: 'hard-drive',
        slot: 'primary',
        run: ({ nav }) => nav.go('hardware', 'disk'),
      },
      ...more,
    ]
  }

  if (tile.opens.kind === 'none') {
    return [
      {
        id: 'nothing-to-open',
        label: openLabel(tile),
        icon: 'circle-help',
        slot: 'primary',
        run: ({ notice }) => notice.info('Set what this opens in hardware.toml'),
      },
      ...more,
    ]
  }

  return [
    {
      id: 'open-tool',
      label: openLabel(tile),
      icon: tile.opens.kind === 'url' ? 'external-link' : 'app-window',
      slot: 'primary',
      async run(ctx) {
        await ctx.api.openHardware(tile.id)
        await hideAfterSuccess(ctx)
      },
    },
    ...more,
  ]
}
