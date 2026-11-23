// Asks the backend whether the folders in a path field exist (api.checkPaths), 300 ms after the
// last keystroke.
import { useDebounceFn } from '@vueuse/core'
import { ref, watch, type Ref } from 'vue'
import { api } from '@/data'
import { splitPaths } from './settingValues'

const DEBOUNCE_MS = 300

/**
 * true = every folder exists, false = one does not (or is a file), undefined = no glyph: the field is
 * empty (the row shows its placeholder), or the answer is not known (yet, or the backend does not answer).
 * All path rows of Settings are folders, so "found" means it exists and is a folder.
 */
export function usePathCheck(text: Ref<string>, enabled?: Ref<boolean>): Ref<boolean | undefined> {
  const found = ref<boolean>()
  /** The newest question. An older answer that arrives later is ignored. */
  let latest = 0
  async function run(): Promise<void> {
    const paths = splitPaths(text.value)
    const mine = ++latest
    // A row that is not a path field (a switch, a choice, a number) never asks the backend.
    if (paths.length === 0 || enabled?.value === false) {
      found.value = undefined
      return
    }
    try {
      const results = await api.checkPaths(paths)
      if (mine === latest) found.value = results.every((result) => result.exists && result.kind === 'dir')
    } catch {
      if (mine === latest) found.value = undefined
    }
  }
  const later = useDebounceFn(run, DEBOUNCE_MS)
  void run() // the first answer does not wait
  watch([text, () => enabled?.value], () => void later())
  return found
}
