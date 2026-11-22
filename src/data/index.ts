// The app's single data entry point. Screens import `api` from here and never
// from the mock folder. Later this becomes createRestApi(baseUrl).

import type { RainApi } from './api'
import { createMockApi } from './mock'

export const api: RainApi = createMockApi()
