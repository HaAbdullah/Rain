import { describe, expect, it } from 'vitest'
import { createMockApi } from './index'

describe('createMockApi', () => {
  it('does not implement getHealth: it is retired from the mock, so the router must send it to the backend', () => {
    const mock: Record<string, unknown> = createMockApi()
    expect(mock.getHealth).toBeUndefined()
    expect(typeof mock.getSettings).toBe('function')
  })
})
