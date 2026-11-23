// The first load after game mode shows what it did, once (section 06 6.7.7).
import { watch } from 'vue'
import { api } from '@/data'
import { recapText } from '@/features/game-mode/recap'
import { shell } from '@/shell'
import { useNotice } from './useNotice'
import { useResource } from './useResource'

export function useGameModeRecap(): void {
  const mode = useResource('gameMode', () => api.getGameMode(), { live: 'gameMode' })
  const notice = useNotice()
  let shownFor = ''
  watch(
    [() => mode.data.value?.last, shell.hidden],
    ([last, hidden]) => {
      if (!last || last.seen || hidden || last.at === shownFor) return
      shownFor = last.at // the mark call below is not instant
      notice.info(recapText(last))
      void api.markGameModeRecapSeen().catch(() => {})
    },
    { immediate: true },
  )
}
