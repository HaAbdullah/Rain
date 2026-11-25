// A small markdown reader for the Claude library. It turns text into plain data (blocks and
// inline pieces); MarkdownBody.ts turns that data into Vue nodes. Nothing here makes HTML, so a
// file can never put markup, a script or a link that opens by itself on the screen.
//
// Supported: headings, paragraphs, bullet and numbered lists (nested by indent), fenced code,
// block quotes, rules, simple tables, `code`, **bold**, *italic* and [links](url). Anything else
// stays as plain text.

export type Inline =
  | { t: 'text'; text: string }
  | { t: 'code'; text: string }
  | { t: 'strong'; children: Inline[] }
  | { t: 'em'; children: Inline[] }
  // A link is shown as text. The address is kept for the tooltip only.
  | { t: 'link'; children: Inline[]; href: string }

export interface ListItem {
  children: Inline[]
  /** Lists nested below this item. */
  sub: Block[]
}

export type Block =
  | { t: 'heading'; level: number; children: Inline[] }
  | { t: 'paragraph'; children: Inline[] }
  | { t: 'code'; lang: string; text: string }
  | { t: 'list'; ordered: boolean; items: ListItem[] }
  | { t: 'quote'; children: Block[] }
  | { t: 'rule' }
  | { t: 'table'; head: Inline[][]; rows: Inline[][][] }

/** Nesting deeper than this is read as plain text (a quote in a quote in a quote ...). */
const MAX_DEPTH = 6

// ---- inline ----

/** The index of the closing `mark` after `from`, or -1. */
function closeOf(text: string, mark: string, from: number): number {
  const at = text.indexOf(mark, from)
  return at <= from ? -1 : at
}

/** A longer piece is shown as plain text: scanning it for marks could take too long. */
const INLINE_MAX = 20000

export function parseInline(text: string, depth = 0): Inline[] {
  if (text.length > INLINE_MAX) return [{ t: 'text', text }]
  const out: Inline[] = []
  let plain = ''
  const flush = () => {
    if (plain !== '') out.push({ t: 'text', text: plain })
    plain = ''
  }
  let i = 0
  while (i < text.length) {
    const c = text[i]
    // A backslash keeps the next mark as it is.
    if (c === '\\' && i + 1 < text.length && '\\`*_[]()#'.includes(text[i + 1])) {
      plain += text[i + 1]
      i += 2
      continue
    }
    if (c === '`') {
      const ticks = text.startsWith('``', i) ? '``' : '`'
      const end = closeOf(text, ticks, i + ticks.length)
      if (end > 0) {
        flush()
        out.push({ t: 'code', text: text.slice(i + ticks.length, end).trim() })
        i = end + ticks.length
        continue
      }
    }
    if (depth < MAX_DEPTH && (text.startsWith('**', i) || text.startsWith('__', i))) {
      const mark = text.slice(i, i + 2)
      const end = closeOf(text, mark, i + 2)
      if (end > 0) {
        flush()
        out.push({ t: 'strong', children: parseInline(text.slice(i + 2, end), depth + 1) })
        i = end + 2
        continue
      }
    }
    if (depth < MAX_DEPTH && (c === '*' || c === '_') && text[i + 1] !== ' ') {
      // An underscore inside a word (snake_case) is not an emphasis mark.
      const insideWord = c === '_' && i > 0 && /[A-Za-z0-9]/.test(text[i - 1])
      const end = insideWord ? -1 : closeOf(text, c, i + 1)
      if (end > 0 && text[end - 1] !== ' ') {
        flush()
        out.push({ t: 'em', children: parseInline(text.slice(i + 1, end), depth + 1) })
        i = end + 1
        continue
      }
    }
    if (depth < MAX_DEPTH && c === '[') {
      const middle = text.indexOf('](', i + 1)
      const end = middle > 0 ? text.indexOf(')', middle + 2) : -1
      if (middle > 0 && end > 0) {
        flush()
        out.push({
          t: 'link',
          children: parseInline(text.slice(i + 1, middle), depth + 1),
          href: text.slice(middle + 2, end).trim(),
        })
        i = end + 1
        continue
      }
    }
    plain += c
    i += 1
  }
  flush()
  return out
}

// ---- blocks ----

const FENCE = /^\s{0,3}(`{3,}|~{3,})\s*([^\s`]*)/
const HEADING = /^\s{0,3}(#{1,6})\s+(.*?)\s*#*\s*$/
const RULE = /^\s{0,3}([-*_])(\s*\1){2,}\s*$/
const BULLET = /^(\s*)[-*+]\s+(.*)$/
const NUMBERED = /^(\s*)\d+[.)]\s+(.*)$/
const QUOTE = /^\s{0,3}>\s?(.*)$/
const TABLE_ROW = /^\s*\|.*\|\s*$/
const TABLE_RULE = /^\s*\|?\s*:?-{2,}:?\s*(\|\s*:?-{2,}:?\s*)*\|?\s*$/

function isBlank(line: string): boolean {
  return line.trim() === ''
}

function cells(line: string): string[] {
  return line
    .trim()
    .replace(/^\|/, '')
    .replace(/\|$/, '')
    .split('|')
    .map((cell) => cell.trim())
}

/** True when the line starts something other than a paragraph. */
function startsBlock(line: string): boolean {
  return (
    FENCE.test(line) || HEADING.test(line) || RULE.test(line) || BULLET.test(line) || NUMBERED.test(line) || QUOTE.test(line)
  )
}

/** The width of the indent, a tab counting as four spaces. */
function indentOf(spaces: string): number {
  return spaces.replace(/\t/g, '    ').length
}

export function parseMarkdown(source: string, depth = 0): Block[] {
  const lines = source.replace(/\r\n?/g, '\n').split('\n')
  const blocks: Block[] = []
  let i = 0
  while (i < lines.length) {
    const line = lines[i]
    if (isBlank(line)) {
      i += 1
      continue
    }

    const fence = FENCE.exec(line)
    if (fence) {
      const body: string[] = []
      i += 1
      // An unclosed fence runs to the end of the text.
      while (i < lines.length && !lines[i].trimStart().startsWith(fence[1])) {
        body.push(lines[i])
        i += 1
      }
      i += 1
      blocks.push({ t: 'code', lang: fence[2], text: body.join('\n') })
      continue
    }

    const heading = HEADING.exec(line)
    if (heading) {
      blocks.push({ t: 'heading', level: heading[1].length, children: parseInline(heading[2]) })
      i += 1
      continue
    }

    if (RULE.test(line)) {
      blocks.push({ t: 'rule' })
      i += 1
      continue
    }

    if (TABLE_ROW.test(line) && i + 1 < lines.length && TABLE_RULE.test(lines[i + 1])) {
      const head = cells(line).map((cell) => parseInline(cell))
      const rows: Inline[][][] = []
      i += 2
      while (i < lines.length && TABLE_ROW.test(lines[i])) {
        rows.push(cells(lines[i]).map((cell) => parseInline(cell)))
        i += 1
      }
      blocks.push({ t: 'table', head, rows })
      continue
    }

    if (QUOTE.test(line)) {
      const inner: string[] = []
      while (i < lines.length && QUOTE.test(lines[i])) {
        inner.push(QUOTE.exec(lines[i])![1])
        i += 1
      }
      blocks.push({ t: 'quote', children: depth < MAX_DEPTH ? parseMarkdown(inner.join('\n'), depth + 1) : [{ t: 'paragraph', children: [{ t: 'text', text: inner.join(' ') }] }] })
      continue
    }

    if (BULLET.test(line) || NUMBERED.test(line)) {
      const taken = readList(lines, i)
      blocks.push(taken.block)
      i = taken.next
      continue
    }

    // A paragraph runs until a blank line or the start of another block.
    const text: string[] = [line.trim()]
    i += 1
    while (i < lines.length && !isBlank(lines[i]) && !startsBlock(lines[i])) {
      text.push(lines[i].trim())
      i += 1
    }
    blocks.push({ t: 'paragraph', children: parseInline(text.join(' ')) })
  }
  return blocks
}

interface ListLine {
  indent: number
  ordered: boolean
  text: string
}

function listLine(line: string): ListLine | null {
  const bullet = BULLET.exec(line)
  if (bullet) return { indent: indentOf(bullet[1]), ordered: false, text: bullet[2] }
  const numbered = NUMBERED.exec(line)
  if (numbered) return { indent: indentOf(numbered[1]), ordered: true, text: numbered[2] }
  return null
}

/** Reads one list (and the lists nested in it) that starts at line `start`. */
function readList(lines: string[], start: number): { block: Block; next: number } {
  const entries: ListLine[] = []
  let i = start
  while (i < lines.length) {
    const entry = listLine(lines[i])
    if (entry) {
      entries.push(entry)
      i += 1
    } else if (!isBlank(lines[i]) && entries.length > 0 && /^\s+\S/.test(lines[i]) && !startsBlock(lines[i])) {
      // A continuation line of the last item.
      entries[entries.length - 1].text += ` ${lines[i].trim()}`
      i += 1
    } else if (isBlank(lines[i]) && i + 1 < lines.length && listLine(lines[i + 1])) {
      // One blank line between two items does not end the list.
      i += 1
    } else {
      break
    }
  }
  return { block: buildList(entries, 0, entries.length, 0), next: i }
}

/** Turns the entries from `from` to `to` into a list; deeper entries become the lists of an item. */
function buildList(entries: ListLine[], from: number, to: number, depth: number): Block {
  const base = entries[from].indent
  const items: ListItem[] = []
  let k = from
  while (k < to) {
    const entry = entries[k]
    let end = k + 1
    // Below the depth limit the deeper lines stay items of this list.
    while (depth < MAX_DEPTH && end < to && entries[end].indent > base) end += 1
    items.push({
      children: parseInline(entry.text.replace(/^\[( |x|X)\]\s+/, (_m, mark: string) => (mark === ' ' ? '[ ] ' : '[x] '))),
      sub: end > k + 1 ? [buildList(entries, k + 1, end, depth + 1)] : [],
    })
    k = end
  }
  return { t: 'list', ordered: entries[from].ordered, items }
}

/** The plain text of inline pieces, for a tooltip or a test. */
export function inlineText(parts: Inline[]): string {
  return parts
    .map((part) => (part.t === 'text' || part.t === 'code' ? part.text : inlineText(part.children)))
    .join('')
}
