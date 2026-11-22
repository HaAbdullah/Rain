// Picks the Shell once, at start-up. Everything else imports `shell` from here.
import { createBrowserShell } from './browser'
import type { Shell } from './shell'

export type { Shell } from './shell'

// The same test as isTauri() from @tauri-apps/api/core. It is repeated here so
// that the Tauri package is not part of what a plain browser loads.
const inTauri = (globalThis as { isTauri?: boolean }).isTauri === true

// Top-level await: modules that import `shell` wait for this line, so the real
// window state is known before the first component is set up.
export const shell: Shell = inTauri
  ? await (await import('./tauri')).createTauriShell()
  : createBrowserShell()
