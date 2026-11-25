// The colour theme of the window. The choice lives in localStorage, not in the backend's
// settings file: it belongs to this window. The inline script in index.html applies the stored
// theme before the first paint; this module keeps it in step afterwards.
//
// A built-in theme is a set of the same tokens in src/styles/tokens.css, picked by data-theme on
// <html>. A theme from VS Code has no block there: its colour tokens come from the backend
// (GET /themes/vscode/theme) and are set as CSS variables on <html> (data-theme="vscode"). The
// resolved values are stored too, so the next start paints at once without asking the backend.
// html also carries class="dark" in every dark theme, for the stock components' dark: variants.
//
// Storage keys:
//   rain.theme          a built-in id, or "vscode"
//   rain.theme.vscode   JSON { id, label, dark, tokens } of the applied VS Code theme
//   rain.theme.follow   "1" while "Follow VS Code" is on
import { readonly, ref } from 'vue'
import type { VsCodeThemeTokens } from '@/data/types'

export const THEME_KEY = 'rain.theme'
export const VSCODE_THEME_KEY = 'rain.theme.vscode'
export const FOLLOW_KEY = 'rain.theme.follow'

export type ThemeId = 'dark' | 'high-contrast' | 'dim' | 'midnight' | 'light'
/** What is in use: a built-in theme, or a theme taken from VS Code. */
export type ActiveTheme = ThemeId | 'vscode'

export interface ThemeInfo {
  id: ThemeId
  name: string
  /** The three colours of the picker card: a surface, the text on it, and the accent. Same as the tokens. */
  swatch: { surface: string; text: string; accent: string }
}

/** The VS Code theme that is applied: who it is, and its colour tokens (without the leading --). */
export interface AppliedVsCodeTheme {
  id: string
  label: string
  dark: boolean
  tokens: Record<string, string>
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

function read(key: string): string | null {
  try {
    return globalThis.localStorage?.getItem(key) ?? null
  } catch {
    return null
  }
}

function write(key: string, value: string | null) {
  try {
    if (value === null) globalThis.localStorage?.removeItem?.(key)
    else globalThis.localStorage?.setItem(key, value)
  } catch {
    // Storage may be blocked: the theme still holds until the window closes.
  }
}

/** Only plain names and values go onto the page: a custom property name, and text without braces. */
export function cleanTokens(value: unknown): Record<string, string> {
  const out: Record<string, string> = {}
  if (typeof value !== 'object' || value === null) return out
  for (const [name, text] of Object.entries(value)) {
    if (/^[a-z0-9-]+$/.test(name) && typeof text === 'string' && text.length < 200 && !/[;{}<>]/.test(text)) {
      out[name] = text
    }
  }
  return out
}

/** The VS Code theme saved by the last run, or null when there is none or it is damaged. */
function readStoredVsCode(): AppliedVsCodeTheme | null {
  const text = read(VSCODE_THEME_KEY)
  if (!text) return null
  try {
    const value = JSON.parse(text) as Partial<AppliedVsCodeTheme> | null
    const tokens = cleanTokens(value?.tokens)
    if (!value || typeof value.id !== 'string' || typeof value.label !== 'string' || !tokens.background) return null
    return { id: value.id, label: value.label, dark: value.dark !== false, tokens }
  } catch {
    return null
  }
}

function readStored(): { active: ActiveTheme; vscode: AppliedVsCodeTheme | null } {
  const stored = read(THEME_KEY)
  if (stored === 'vscode') {
    const vscode = readStoredVsCode()
    if (vscode) return { active: 'vscode', vscode }
    return { active: DEFAULT_THEME, vscode: null }
  }
  return { active: isThemeId(stored) ? stored : DEFAULT_THEME, vscode: readStoredVsCode() }
}

/** Removes the custom properties that an earlier VS Code theme set on the root element. */
function clearTokens(root: HTMLElement) {
  const style = root.style
  if (!style || typeof style.item !== 'function') return
  for (let at = style.length - 1; at >= 0; at--) {
    const name = style.item(at)
    if (name.startsWith('--')) style.removeProperty(name)
  }
  style.removeProperty('color-scheme')
}

function applyBuiltin(id: ThemeId) {
  const root = globalThis.document?.documentElement
  if (!root) return
  clearTokens(root)
  root.dataset.theme = id
  root.classList.toggle('dark', id !== 'light')
}

function applyVsCode(applied: AppliedVsCodeTheme) {
  const root = globalThis.document?.documentElement
  if (!root) return
  clearTokens(root)
  root.dataset.theme = 'vscode'
  root.classList.toggle('dark', applied.dark)
  const style = root.style
  if (!style) return
  for (const [name, value] of Object.entries(applied.tokens)) style.setProperty(`--${name}`, value)
  style.setProperty('color-scheme', applied.dark ? 'dark' : 'light')
}

const initial = readStored()
const current = ref<ActiveTheme>(initial.active)
const vscodeApplied = ref<AppliedVsCodeTheme | null>(initial.vscode)
const following = ref(read(FOLLOW_KEY) === '1')
/** Why the last check of VS Code did not change the theme, for the picker to show. Empty when all is well. */
const followProblem = ref('')

if (initial.active === 'vscode' && initial.vscode) applyVsCode(initial.vscode)
else applyBuiltin(initial.active === 'vscode' ? DEFAULT_THEME : initial.active)

/** The theme now in use: a built-in id, or "vscode". */
export const theme = readonly(current)
/** The applied VS Code theme (it is kept while a built-in theme is chosen, until another replaces it). */
export const vscodeTheme = readonly(vscodeApplied)
/** True while "Follow VS Code" is on. */
export const followVsCode = readonly(following)
export const followVsCodeProblem = readonly(followProblem)

/** Switches to a built-in theme at once and remembers it. An unknown id is ignored. Follow VS Code goes off. */
export function setTheme(id: ThemeId) {
  if (!isThemeId(id)) return
  current.value = id
  applyBuiltin(id)
  write(THEME_KEY, id)
  setFollow(false)
}

function setFollow(on: boolean) {
  following.value = on
  followProblem.value = ''
  write(FOLLOW_KEY, on ? '1' : '0')
}

/** Applies the tokens of a VS Code theme, and remembers them for the next start. */
function adopt(theme: VsCodeThemeTokens) {
  const tokens = cleanTokens(theme.tokens)
  if (!tokens.background) throw new Error('The theme has no colours')
  const applied: AppliedVsCodeTheme = { id: theme.id, label: theme.label, dark: theme.dark, tokens }
  vscodeApplied.value = applied
  current.value = 'vscode'
  applyVsCode(applied)
  write(THEME_KEY, 'vscode')
  write(VSCODE_THEME_KEY, JSON.stringify(applied))
}

/** The api is loaded when it is needed, so this module stays free of the shell (and testable). */
async function api() {
  return (await import('@/data')).api
}

/** Picks one VS Code theme by hand. Follow VS Code goes off. Rejects when the backend cannot read it. */
export async function chooseVsCodeTheme(id: string) {
  adopt(await (await api()).getVsCodeTheme(id))
  setFollow(false)
}

let checking: Promise<void> | null = null

/**
 * "Follow VS Code": asks which theme VS Code uses now and applies it when it differs from the
 * one in use. It does nothing when the switch is off. It reads files only when called: at
 * start, when the window is summoned and when Settings opens, never on a timer.
 */
export function checkFollowVsCode(): Promise<void> {
  if (!following.value) return Promise.resolve()
  checking ??= (async () => {
    try {
      const client = await api()
      const now = await client.getVsCodeCurrentTheme()
      if (!following.value) return
      if (!now.id) {
        followProblem.value = now.label
          ? `VS Code uses "${now.label}", which is not an installed theme extension`
          : 'VS Code has no theme set'
        return
      }
      followProblem.value = ''
      if (current.value === 'vscode' && vscodeApplied.value?.id === now.id) return
      adopt(await client.getVsCodeTheme(now.id))
    } catch {
      followProblem.value = 'Could not read the VS Code theme'
    } finally {
      checking = null
    }
  })()
  return checking
}

/** Turns "Follow VS Code" on or off. On: the current VS Code theme is applied at once. */
export async function setFollowVsCode(on: boolean) {
  setFollow(on)
  if (on) await checkFollowVsCode()
}

// Start: when the window follows VS Code, look once. (A summon and the Settings screen look again.)
if (following.value) void checkFollowVsCode()

export function useTheme() {
  return { theme, themes, setTheme }
}
