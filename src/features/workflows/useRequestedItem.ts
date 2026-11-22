// "Show in Workflows" from Home arrives as route.params.id: select that item once it is in the list.
import { watch } from 'vue'
import { useNav } from '@/nav/useNav'

export function useRequestedItem(ids: () => string[], select: (id: string) => void): void {
  const wanted = useNav().route.value.params?.id
  if (wanted === undefined) return

  let done = false
  watch(
    ids,
    (all) => {
      if (done || !all.includes(wanted)) return
      done = true
      select(wanted)
    },
    { immediate: true },
  )
}
