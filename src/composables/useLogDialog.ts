// Opener for the log viewer (job runs, an idle job's log, a session transcript).
// The state lives here; the LogDialog component (mounted once in AppShell) draws it.
import { ref, shallowRef, type Ref } from 'vue'
import type { ID, JobRun, LogLine } from '@/data/types'
import { useOverlay } from './useOverlay'

export interface LogRequest {
  title: string
  /** With runs the dialog shows the run list on the left and loads one log per run. */
  runs?: JobRun[]
  selectedRunId?: ID
  /** Called with the selected run's id, or with nothing when there is no run list. */
  load: (runId?: ID) => Promise<LogLine[]>
}

const isOpen = ref(false)
const request = shallowRef<LogRequest | null>(null)

useOverlay(isOpen)

export interface LogDialogStore {
  isOpen: Ref<boolean>
  request: Readonly<Ref<LogRequest | null>>
  open(request: LogRequest): void
  close(): void
}

const store: LogDialogStore = {
  isOpen,
  request,
  open(next) {
    request.value = next
    isOpen.value = true
  },
  close() {
    isOpen.value = false
  },
}

export function useLogDialog(): LogDialogStore {
  return store
}
