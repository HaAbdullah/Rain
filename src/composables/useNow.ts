// A clock for relative times ("2m", "in 9 h"). It ticks only while the window
// is visible, so a hidden Rain runs no interval at all (spec 2.6).
import { getCurrentScope, onScopeDispose, ref, watch, type Ref } from 'vue'
import { shell } from '@/shell'

export function useNow(stepMs = 30_000): Ref<number> {
  const now = ref(Date.now())
  let timer: number | undefined

  function stop() {
    window.clearInterval(timer)
    timer = undefined
  }

  const stopWatching = watch(
    shell.hidden,
    (hidden) => {
      stop()
      if (hidden) return
      // Catch up at once: the window may have been hidden for hours.
      now.value = Date.now()
      timer = window.setInterval(() => (now.value = Date.now()), stepMs)
    },
    { immediate: true },
  )

  if (getCurrentScope()) {
    onScopeDispose(() => {
      stopWatching()
      stop()
    })
  }

  return now
}
