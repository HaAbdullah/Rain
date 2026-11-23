// What the backend says about its sources (Health.sources): a source that is off or failed has a
// short text, for example "Steam is not installed". A screen shows that text as a quiet line
// where the source's data would be; it never turns it into an error or a spinner.
import { computed, type ComputedRef } from 'vue'
import { api } from '@/data'
import type { SourceStatus } from '@/data/types'
import { useResource } from './useResource'

/** The sources among `names` that are not ok and have something to say. An unknown health answer gives none. */
export function useSourceNotes(...names: string[]): ComputedRef<SourceStatus[]> {
  // The same key and topic as Settings: one fetch feeds both.
  const health = useResource('health', () => api.getHealth(), { live: 'health' })
  return computed(() =>
    (health.data.value?.sources ?? []).filter(
      (source) => names.includes(source.name) && source.state !== 'ok' && source.message,
    ),
  )
}
