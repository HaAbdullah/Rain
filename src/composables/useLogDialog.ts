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
  /**
   * True while the log of this run is still being written. When it is given, the dialog loads the
   * log again every 3 seconds while this answers true (and while it is open and visible).
   */
  follow?: (runId?: ID) => Promise<boolean>
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
