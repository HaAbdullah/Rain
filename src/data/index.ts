// The app's single data entry point. Screens import `api` from here and never from the mock
// folder or the REST client (foundation 8.5).
//
// While the backend is built stage by stage, `api` is a router: the methods and topics of
// finished stages go to the Rust backend, every other one to the mock (routing.ts).
// `VITE_RAIN_MOCK=1` (npm run dev:ui) runs the whole app on the mock and opens no stream.
// In stage 10 this file becomes `createRestApi(shell.bridge, stream)` alone.
import { shell } from '@/shell'
import type { RainApi } from './api'
import { createMockApi } from './mock'
import { createRestApi } from './rest/client'
import { createStream, type RestStream } from './rest/stream'
import { createRouterApi } from './router'
import { routing } from './routing'

interface DataLayer {
  api: RainApi
  onStreamReset: RestStream['onReset']
}

function createDataLayer(): DataLayer {
  if (import.meta.env.VITE_RAIN_MOCK === '1') {
    // Retired methods are undefined here; their screens show an error row. Stage 10 deletes this branch.
    return { api: createMockApi() as RainApi, onStreamReset: () => () => {} }
  }
  const stream = createStream(shell.bridge)
  const real = createRestApi(shell.bridge, stream)
  return { api: createRouterApi({ real, mock: createMockApi(), routing }), onStreamReset: stream.onReset }
}

export const { api, onStreamReset } = createDataLayer()
