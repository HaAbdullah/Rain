// Turns a key combo from the keymap ('Ctrl+KeyK', 'Shift+Escape') into the
// keycap labels shown by KeyHint, the tooltips and the shortcut sheet.

/** Labels for `KeyboardEvent.code` values and modifier names that are not shown as written. */
const CAPS: Record<string, string> = {
  Control: 'Ctrl',
  Escape: 'Esc',
  Enter: '↵',
  Backspace: '⌫',
  Tab: '⇥',
  ArrowUp: '↑',
  ArrowDown: '↓',
  ArrowLeft: '←',
  ArrowRight: '→',
  Up: '↑',
  Down: '↓',
  Left: '←',
  Right: '→',
  Backquote: '`',
  Comma: ',',
  Period: '.',
  Slash: '/',
  Minus: '-',
  Equal: '=',
  Plus: '+',
  PageUp: 'PgUp',
  PageDown: 'PgDn',
  Delete: 'Del',
  MouseBack: 'Mouse back',
  MouseForward: 'Mouse forward',
  BrowserBack: 'Back key',
  BrowserForward: 'Forward key',
}

function cap(key: string): string {
  if (key in CAPS) return CAPS[key]
  // Physical key codes: 'KeyK' -> 'K', 'Digit1' -> '1'.
  const code = /^(?:Key|Digit)(.)$/.exec(key)
  return code ? code[1] : key
}

/** 'Ctrl+Enter' -> ['Ctrl', '↵'], 'Ctrl+KeyK' -> ['Ctrl', 'K'], 'F11' -> ['F11']. */
export function formatCombo(combo: string): string[] {
  return combo
    .split('+')
    .map((part) => part.trim())
    .filter((part) => part !== '')
    .map(cap)
}
