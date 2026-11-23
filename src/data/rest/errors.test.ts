import { describe, expect, it } from 'vitest'
import { toRainError } from './errors'

describe('toRainError', () => {
  it('passes an object with a code and a message through unchanged', () => {
    const error = { code: 'conflict', message: 'Busy', source: 'jobs', reason: 'busy' }
    expect(toRainError(error)).toBe(error)
  })

  it('uses the message of an Error', () => {
    expect(toRainError(new Error('boom'))).toEqual({ code: 'failed', message: 'boom' })
  })

  it('uses the text of a string', () => {
    expect(toRainError('the shell said no')).toEqual({ code: 'failed', message: 'the shell said no' })
  })

  it('does not trust an object whose code or message is not text', () => {
    expect(toRainError({ code: 500, message: 'x' }).code).toBe('failed')
    expect(toRainError({ code: 'failed' }).message).toBe('[object Object]')
  })

  it('handles null and undefined', () => {
    expect(toRainError(null)).toEqual({ code: 'failed', message: 'null' })
    expect(toRainError(undefined)).toEqual({ code: 'failed', message: 'undefined' })
  })
})
