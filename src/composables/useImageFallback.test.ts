import { ref } from 'vue'
import { describe, expect, it } from 'vitest'
import { useImageFallback } from './useImageFallback'

describe('useImageFallback', () => {
  it('starts without a failure and fails when the image reports an error', () => {
    const { failed, onError } = useImageFallback(() => 'http://host/media/app-icon/a')
    expect(failed.value).toBe(false)
    onError()
    expect(failed.value).toBe(true)
  })

  it('tries again when the address changes', () => {
    const address = ref<string | undefined>('http://host/media/game-cover/g1?v=1')
    const { failed, onError } = useImageFallback(() => address.value)
    onError()

    address.value = 'http://host/media/game-cover/g1?v=2'
    expect(failed.value).toBe(false)
  })

  it('keeps the failure while the address stays the same', () => {
    const address = ref<string | undefined>('http://host/media/game-cover/g1?v=1')
    const { failed, onError } = useImageFallback(() => address.value)
    onError()

    address.value = 'http://host/media/game-cover/g1?v=1'
    expect(failed.value).toBe(true)
  })
})
