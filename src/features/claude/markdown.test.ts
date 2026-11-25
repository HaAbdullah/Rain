import { describe, expect, it } from 'vitest'
import { h } from 'vue'
import { renderToString } from 'vue/server-renderer'
import MarkdownBody from './MarkdownBody'
import { inlineText, parseInline, parseMarkdown } from './markdown'

describe('inline marks', () => {
  it('reads code, bold, italic and links', () => {
    const parts = parseInline('a `x` **b** *c* [d](http://e.test)')
    expect(parts.map((part) => part.t)).toEqual(['text', 'code', 'text', 'strong', 'text', 'em', 'text', 'link'])
    expect(parts[7]).toMatchObject({ t: 'link', href: 'http://e.test' })
  })

  it('keeps a snake_case word and a lone star as text', () => {
    expect(inlineText(parseInline('use my_var_name and 2 * 3'))).toBe('use my_var_name and 2 * 3')
    expect(parseInline('use my_var_name').every((part) => part.t === 'text')).toBe(true)
  })

  it('leaves a mark that never closes as text', () => {
    expect(parseInline('**open and `tick')).toEqual([{ t: 'text', text: '**open and `tick' }])
  })

  it('does not scan a huge piece for marks', () => {
    const huge = '['.repeat(50000)
    expect(parseInline(huge)).toEqual([{ t: 'text', text: huge }])
  })
})

describe('blocks', () => {
  it('reads headings, paragraphs, rules and fenced code', () => {
    const blocks = parseMarkdown('# Title\n\nsome\ntext\n\n---\n\n```ts\nlet a = 1\n\nlet b = 2\n```\n')
    expect(blocks.map((b) => b.t)).toEqual(['heading', 'paragraph', 'rule', 'code'])
    expect(blocks[1]).toMatchObject({ t: 'paragraph' })
    expect(blocks[3]).toEqual({ t: 'code', lang: 'ts', text: 'let a = 1\n\nlet b = 2' })
  })

  it('runs an unclosed fence to the end', () => {
    expect(parseMarkdown('```\nabc\ndef')).toEqual([{ t: 'code', lang: '', text: 'abc\ndef' }])
  })

  it('nests lists by indent and numbers them', () => {
    const [list] = parseMarkdown('- one\n  - inner\n- two\n')
    expect(list).toMatchObject({ t: 'list', ordered: false })
    if (list.t !== 'list') throw new Error('not a list')
    expect(list.items).toHaveLength(2)
    expect(list.items[0].sub).toHaveLength(1)
    const [numbered] = parseMarkdown('1. a\n2. b')
    expect(numbered).toMatchObject({ t: 'list', ordered: true })
  })

  it('reads a table and a quote', () => {
    const blocks = parseMarkdown('| a | b |\n|---|---|\n| 1 | 2 |\n\n> quoted **text**')
    expect(blocks[0]).toMatchObject({ t: 'table' })
    expect(blocks[1]).toMatchObject({ t: 'quote' })
  })

  it('survives deep nesting', () => {
    const lines = Array.from({ length: 200 }, (_, n) => `${' '.repeat(n * 2)}- item ${n}`).join('\n')
    expect(() => parseMarkdown(lines)).not.toThrow()
    const quotes = `${'> '.repeat(100)}deep`
    expect(() => parseMarkdown(quotes)).not.toThrow()
  })
})

describe('MarkdownBody', () => {
  it('draws text nodes only: markup in the file is shown as text', async () => {
    const html = await renderToString(
      h(MarkdownBody, {
        source: ['# Hi', '', '<script>alert(1)</script> <img src=x onerror=alert(1)>', '', '[link](javascript:alert(1))'].join(
          String.fromCharCode(10),
        ),
      }),
    )
    // The tags of the file are escaped, so no script or image element exists.
    expect(html).not.toContain('<script')
    expect(html).not.toContain('<img')
    expect(html).not.toContain('<a ')
    expect(html).toContain('&lt;script&gt;alert(1)&lt;/script&gt;')
    expect(html).toContain('<h2')
  })
})
