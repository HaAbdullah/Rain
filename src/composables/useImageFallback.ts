// Tells a component that its image did not load, so it can draw the letter tile instead
// (foundation 4.7: a missing icon or cover is a 404 and the screens already have a fallback).
// The flag is cleared when the address changes, because a new image deserves a new try.
import { ref, watch } from 'vue'

export function useImageFallback(source: () => string | undefined) {
  const failed = ref(false)
  watch(source, () => {
    failed.value = false
  }, { flush: 'sync' })
  return {
    failed,
    /** Bind it to the `error` event of the `<img>`. */
    onError() {
      failed.value = true
    },
  }
}
