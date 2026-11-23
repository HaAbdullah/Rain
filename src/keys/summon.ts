// What happens around hiding and summoning the window (spec 2.2, 2.4, 2.6).
// The key that summons is a global hotkey in Rust; this file is the frontend's
// reaction to it, and it applies equally to the browser shell's simulated hide.
import { nextTick, onMounted, onScopeDispose, watch } from 'vue'
import { refreshResources, resetResources, useResource } from '@/composables/useResource'
import { useSearch } from '@/composables/useSearch'
import { resetViewState } from '@/composables/useViewState'
import { api } from '@/data'
import { settingDefaults } from '@/features/settings/actions'
import { useNav } from '@/nav/useNav'
import { shell } from '@/shell'

/** Call once, from the component that owns the app frame. */
export function useSummon(): void {
  const search = useSearch()
  const nav = useNav()
  const settings = useResource('settings', () => api.getSettings(), { live: 'settings' })
  let resetTimer: number | undefined

  /** After a long hide Rain starts over: Home, first row, nothing remembered. */
  function resetToHome() {
    nav.go('home')
    resetViewState()
    resetResources()
  }

  const stopOnHide = shell.onHide(() => {
    const seconds = settings.data.value?.resetAfterSeconds ?? settingDefaults.resetAfterSeconds
    // One of the two one-shot timers that may run while hidden (spec 2.6).
    resetTimer = window.setTimeout(resetToHome, seconds * 1000)
  })

  const stopOnSummon = shell.onSummon(() => {
    window.clearTimeout(resetTimer)
    // Every summon starts with an empty, unscoped search field, so
    // "summon, type, Enter" means the same thing on every screen.
    search.setScope(null)
    search.clear()
    refreshResources()
    // After the next render: in the browser the frame comes back from behind the placeholder first.
    void nextTick(() => search.focus())
  })

  // CSS pauses the spinner under html[data-hidden].
  watch(
    shell.hidden,
    (hidden) => document.documentElement.toggleAttribute('data-hidden', hidden),
    { immediate: true },
  )

  // "Hide when focus is lost" switched off is one more reason to stay open.
  let releaseSetting: (() => void) | null = null
  watch(
    () => settings.data.value?.hideOnBlur,
    (hideOnBlur) => {
      releaseSetting?.()
      releaseSetting = hideOnBlur === false ? shell.holdOpen('setting: hide on blur is off') : null
    },
    { immediate: true },
  )

  onMounted(() => search.focus())

  onScopeDispose(() => {
    window.clearTimeout(resetTimer)
    stopOnHide()
    stopOnSummon()
    releaseSetting?.()
  })
}
