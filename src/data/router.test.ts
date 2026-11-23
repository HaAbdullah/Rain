import { describe, expect, it } from 'vitest'
import type { RainApi, Topic } from './api'
import { createRouterApi } from './router'
import { routing as realRouting, type MethodName, type Routing } from './routing'

/** An api whose every method answers `<label>:<method>:<arguments as JSON>`, so a test sees who answered. */
function labelledApi(label: string): RainApi {
  const methods = Object.fromEntries(
    Object.keys(realRouting.methods).map((name) => [
      name,
      (...args: unknown[]) => Promise.resolve(`${label}:${name}:${JSON.stringify(args)}`),
    ]),
  )
  const subscribe = (topic: Topic, cb: (payload: never) => void) => {
    ;(cb as (payload: string) => void)(`${label}:${topic}`)
    return () => {}
  }
  return { ...methods, subscribe } as unknown as RainApi
}

describe('router', () => {
  const routing: Routing = {
    methods: { ...realRouting.methods, listApps: 'real', listPools: 'mock' },
    topics: { ...realRouting.topics, jobs: 'real', sessions: 'mock' },
  }
  const api = createRouterApi({ real: labelledApi('real'), mock: labelledApi('mock'), routing })

  it('sends a method to the side the table names', async () => {
    expect(await api.listApps()).toBe('real:listApps:[]')
    expect(await api.listPools()).toBe('mock:listPools:[]')
  })

  it('passes the arguments on unchanged', async () => {
    const answer = await api.setPinned('app', 'app-1', true)
    expect(answer).toBe('mock:setPinned:["app","app-1",true]')
  })

  it('sends a topic subscription to the side the table names', () => {
    const seen: unknown[] = []
    api.subscribe('jobs', (payload) => seen.push(payload))
    api.subscribe('sessions', (payload) => seen.push(payload))
    expect(seen).toEqual(['real:jobs', 'mock:sessions'])
  })

  it('reads the table at call time, so a changed table changes the side', async () => {
    const movable: Routing = { ...routing, methods: { ...routing.methods, listApps: 'mock' } }
    const moved = createRouterApi({ real: labelledApi('real'), mock: labelledApi('mock'), routing: movable })
    expect(await moved.listApps()).toBe('mock:listApps:[]')
    movable.methods.listApps = 'real'
    expect(await moved.listApps()).toBe('real:listApps:[]')
  })

  it('has a function for every method of the table', () => {
    for (const name of Object.keys(routing.methods) as MethodName[]) {
      expect(typeof api[name]).toBe('function')
    }
  })
})
