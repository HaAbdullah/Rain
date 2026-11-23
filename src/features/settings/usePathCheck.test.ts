import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { nextTick, ref } from 'vue'
import type { PathCheck } from '@/data/types'

const backend = vi.hoisted(() => ({ checkPaths: vi.fn() }))
vi.mock('@/data', () => ({ api: backend }))

import { usePathCheck } from './usePathCheck'

const dir = (path: string): PathCheck => ({ path, exists: true, kind: 'dir' })

/** Lets the promise of a mocked answer finish. */
const flush = async () => {
  await Promise.resolve()
  await Promise.resolve()
  await nextTick()
}

beforeEach(() => {
  vi.useFakeTimers()
  backend.checkPaths.mockReset()
})
afterEach(() => vi.useRealTimers())

describe('usePathCheck', () => {
  it('asks at once for the first value', async () => {
    backend.checkPaths.mockResolvedValue([dir('C:\\a')])
    const found = usePathCheck(ref('C:\\a'))
    expect(backend.checkPaths).toHaveBeenCalledTimes(1)
    expect(backend.checkPaths).toHaveBeenCalledWith(['C:\\a'])
    await flush()
    expect(found.value).toBe(true)
  })

  it('waits 300 ms after the last keystroke and asks once more', async () => {
    backend.checkPaths.mockResolvedValue([dir('x')])
    const text = ref('C:\\a')
    usePathCheck(text)
    await flush()
    backend.checkPaths.mockClear()

    text.value = 'C:\\ab'
    await nextTick()
    await vi.advanceTimersByTimeAsync(100)
    text.value = 'C:\\abc'
    await nextTick()
    await vi.advanceTimersByTimeAsync(100)
    text.value = 'C:\\abcd'
    await nextTick()
    await vi.advanceTimersByTimeAsync(299)
    expect(backend.checkPaths).not.toHaveBeenCalled()
    await vi.advanceTimersByTimeAsync(2)
    expect(backend.checkPaths).toHaveBeenCalledTimes(1)
    expect(backend.checkPaths).toHaveBeenCalledWith(['C:\\abcd'])
  })

  it('splits the field at semicolons and needs every folder to exist', async () => {
    backend.checkPaths.mockResolvedValue([dir('C:\\a'), { path: 'C:\\b', exists: false }])
    const found = usePathCheck(ref('C:\\a; C:\\b'))
    expect(backend.checkPaths).toHaveBeenCalledWith(['C:\\a', 'C:\\b'])
    await flush()
    expect(found.value).toBe(false)
  })

  it('ignores an older answer that arrives after a newer one', async () => {
    let answerFirst: (value: PathCheck[]) => void = () => {}
    backend.checkPaths.mockImplementationOnce(() => new Promise((resolve) => (answerFirst = resolve)))
    backend.checkPaths.mockResolvedValueOnce([{ path: 'C:\\two', exists: false }])
    const text = ref('C:\\one')
    const found = usePathCheck(text)

    text.value = 'C:\\two'
    await nextTick()
    await vi.advanceTimersByTimeAsync(301)
    await flush()
    expect(found.value).toBe(false)

    answerFirst([dir('C:\\one')]) // too late
    await flush()
    expect(found.value).toBe(false)
  })

  it('treats a file as missing, because every path row is a folder', async () => {
    backend.checkPaths.mockResolvedValue([{ path: 'C:\\f.txt', exists: true, kind: 'file' }])
    const found = usePathCheck(ref('C:\\f.txt'))
    await flush()
    expect(found.value).toBe(false)
  })

  it('shows no glyph when the backend does not answer', async () => {
    backend.checkPaths.mockRejectedValue(new Error('down'))
    const found = usePathCheck(ref('C:\\a'))
    await flush()
    expect(found.value).toBeUndefined()
  })

  it('shows no glyph and sends nothing for an empty field', async () => {
    const text = ref('')
    const found = usePathCheck(text)
    await flush()
    expect(backend.checkPaths).not.toHaveBeenCalled()
    expect(found.value).toBeUndefined()

    text.value = '  ;  '
    await nextTick()
    await vi.advanceTimersByTimeAsync(301)
    expect(backend.checkPaths).not.toHaveBeenCalled()
  })
})

describe('usePathCheck on a row that is not a path field', () => {
  it('never asks the backend', async () => {
    backend.checkPaths.mockReset()
    const found = usePathCheck(ref('pomodoro'), ref(false))
    await flush()
    vi.advanceTimersByTime(1000)
    await flush()
    expect(backend.checkPaths).not.toHaveBeenCalled()
    expect(found.value).toBeUndefined()
  })
})
