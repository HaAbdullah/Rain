// What asks for the user's attention: sessions waiting on a permission and a failed
// job. The header and the icon rail both show it, so one place decides what counts.
import { computed } from 'vue'
import { api } from '@/data'
import { useResource } from './useResource'

export function useNeedsYou() {
  const sessions = useResource('sessions', () => api.listSessions(), { live: 'sessions' })
  const jobs = useResource('jobs', () => api.listJobs(), { live: 'jobs' })

  const waiting = computed(
    () => (sessions.data.value ?? []).filter((session) => session.status === 'waiting').length,
  )
  const hasFailedJob = computed(() =>
    (jobs.data.value ?? []).some((job) => job.recentRuns[0]?.status === 'failed'),
  )
  return { waiting, hasFailedJob }
}
