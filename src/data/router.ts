// TEMPORARY (deleted in stage 10): one RainApi that sends every method and topic to the real
// backend or to the mock, as `routing` says (foundation 8.6). Mixed cases such as a search
// that merges both sides are added here by the stage that needs them.
import type { RainApi } from './api'
import type { MockApi } from './mock'
import type { MethodName, Routing, Side } from './routing'

export interface RouterParts {
  real: RainApi
  mock: MockApi
  routing: Routing
}

type AnyMethod = (...args: unknown[]) => unknown

export function createRouterApi({ real, mock, routing }: RouterParts): RainApi {
  // `mock` lacks the retired methods, so it is not a full RainApi. That is safe because the
  // Routing type does not allow a retired method to be routed to the mock.
  const sideApi = (side: Side): RainApi => (side === 'real' ? real : (mock as RainApi))

  // One forwarding function per method. The table decides the side at call time, so the
  // forwarders need no per-method code; the cast is the price of that (the argument types
  // are exactly those of RainApi, because the names come from the same table).
  const forwarders = Object.fromEntries(
    (Object.keys(routing.methods) as MethodName[]).map((name) => [
      name,
      (...args: unknown[]) => {
        const api = sideApi(routing.methods[name])
        return (api[name] as AnyMethod).apply(api, args)
      },
    ]),
  ) as Omit<RainApi, 'subscribe'>

  return {
    ...forwarders,
    subscribe: (topic, cb) => sideApi(routing.topics[topic]).subscribe(topic, cb),
  }
}
