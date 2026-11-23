// The app's single data entry point. Screens import `api` from here and never from the REST
// client directly (foundation 8.5). There is no sample data: every call goes to the backend,
// and the stream is opened once, when this file is first loaded.
import { shell } from '@/shell'
import type { RainApi } from './api'
import { createRestApi } from './rest/client'
import { createStream } from './rest/stream'

const stream = createStream(shell.bridge)

export const api: RainApi = createRestApi(shell.bridge, stream)
export const onStreamReset = stream.onReset
