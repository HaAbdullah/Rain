// The shared resources of the Workflows screen. The strip, the views, the rail and Home
// all ask for these keys, so one fetch and one live subscription feed them all.
import { useResource } from '@/composables/useResource'
import { api } from '@/data'

export const useSessions = () => useResource('sessions', () => api.listSessions(), { live: 'sessions' })
export const useJobs = () => useResource('jobs', () => api.listJobs(), { live: 'jobs' })
export const useIdleJobs = () => useResource('idleJobs', () => api.listIdleJobs(), { live: 'idleJobs' })
export const useMachine = () => useResource('machine', () => api.getMachineState())
