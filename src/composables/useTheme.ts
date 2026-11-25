// The colour theme of the window. The choice lives in localStorage (key "rain.theme"), not in
// the backend's settings file: it belongs to this window. The inline script in index.html
// applies the stored theme before the first paint; this module keeps it in step afterwards.
//
// A theme is a set of the same tokens in src/styles/tokens.css, picked by data-theme on <html>.
// html also carries class="dark" in every theme but light, for the stock components' dark: variants.
import { readonly, ref } from 'vue'

export const THEME_KEY = 'rain.theme'

export type ThemeId = 'dark' | 'high-contrast' | 'dim' | 'midnight' | 'light'

export interface ThemeInfo {
  id: ThemeId
  name: string
  /** The three colours of the picker card: a surface, the text on it, and the accent. Same as the tokens. */
  swatch: { surface: string; text: string; accent: string }
}

export const DEFAULT_THEME: ThemeId = 'dark'

export const themes: readonly ThemeInfo[] = [
  { id: 'dark', name: 'Dark', swatch: { surface: '#171718', text: '#f4f4f5', accent: '#0a78cc' } },
  { id: 'high-contrast', name: 'High contrast', swatch: { surface: '#0a0a0b', text: '#ffffff', accent: '#1470d6' } },
  { id: 'dim', name: 'Dim', swatch: { surface: '#242932', text: '#eef1f6', accent: '#3a73c9' } },
  { id: 'midnight', name: 'Midnight', swatch: { surface: '#0b1022', text: '#eef1fc', accent: '#c2570c' } },
  { id: 'light', name: 'Light', swatch: { surface: '#f4f5f8', text: '#13151a', accent: '#1060c0' } },
]

function isThemeId(value: unknown): value is ThemeId {
  return themes.some((entry) => entry.id === value)
}

/** The stored theme; the default when nothing is stored, the value is unknown or storage is not available. */
function readStored(): ThemeId {
  try {
    const stored = globalThis.localStorage?.getItem(THEME_KEY)
    return isThemeId(stored) ? stored : DEFAULT_THEME
  } catch {
    return DEFAULT_THEME
  }
}

function apply(id: ThemeId) {
  const root = globalThis.document?.documentElement
  if (!root) return
  root.dataset.theme = id
  root.classList.toggle('dark', id !== 'light')
}

const current = ref<ThemeId>(readStored())
apply(current.value)

/** The theme now in use. */
export const theme = readonly(current)

/** Switches theme at once and remembers it. An unknown id is ignored. */
export function setTheme(id: ThemeId) {
  if (!isThemeId(id)) return
  current.value = id
  apply(id)
  try {
    globalThis.localStorage?.setItem(THEME_KEY, id)
  } catch {
    // Storage may be blocked: the theme still holds until the window closes.
  }
}

export function useTheme() {
  return { theme, themes, setTheme }
}
