// Draws the blocks of markdown.ts as Vue nodes. No v-html anywhere: every piece of text goes in as
// a text node, so a file cannot add markup. A link is drawn as text; its address is the tooltip.
import { defineComponent, h, type PropType, type VNode } from 'vue'
import { parseMarkdown, type Block, type Inline } from './markdown'

function inline(parts: Inline[]): (VNode | string)[] {
  return parts.map((part): VNode | string => {
    switch (part.t) {
      case 'text':
        return part.text
      case 'code':
        return h('code', { class: 'rounded-sm bg-surface-2 px-1 py-px font-mono text-xs text-foreground' }, part.text)
      case 'strong':
        return h('strong', { class: 'font-semibold text-foreground' }, inline(part.children))
      case 'em':
        return h('em', { class: 'italic' }, inline(part.children))
      case 'link':
        return h('span', { class: 'text-foreground underline decoration-text-3 underline-offset-2', title: part.href }, inline(part.children))
    }
  })
}

const HEADING_CLASS: Record<number, string> = {
  1: 'mt-4 text-base font-semibold text-foreground',
  2: 'mt-4 text-sm font-semibold text-foreground',
  3: 'mt-3 text-13 font-semibold text-foreground',
  4: 'mt-3 text-13 font-medium text-foreground',
  5: 'mt-3 text-13 font-medium text-muted-foreground',
  6: 'mt-3 text-xs font-medium text-muted-foreground',
}

function block(node: Block, key: number): VNode {
  switch (node.t) {
    case 'heading':
      return h(`h${Math.min(6, node.level + 1)}`, { key, class: HEADING_CLASS[node.level] }, inline(node.children))
    case 'paragraph':
      return h('p', { key, class: 'text-13 leading-relaxed text-muted-foreground' }, inline(node.children))
    case 'code':
      return h(
        'pre',
        { key, class: 'overflow-x-auto rounded-md bg-surface-2 p-3 font-mono text-xs leading-relaxed text-foreground' },
        h('code', node.text),
      )
    case 'rule':
      return h('hr', { key, class: 'border-border' })
    case 'quote':
      return h('blockquote', { key, class: 'flex flex-col gap-2 border-l-2 border-border pl-3' }, node.children.map(block))
    case 'list':
      return h(
        node.ordered ? 'ol' : 'ul',
        { key, class: `flex flex-col gap-1 pl-5 text-13 text-muted-foreground ${node.ordered ? 'list-decimal' : 'list-disc'}` },
        node.items.map((item, index) =>
          h('li', { key: index, class: 'pl-1' }, [
            ...inline(item.children),
            ...(item.sub.length > 0 ? [h('div', { class: 'mt-1' }, item.sub.map(block))] : []),
          ]),
        ),
      )
    case 'table':
      return h('div', { key, class: 'overflow-x-auto' }, [
        h('table', { class: 'w-full border-collapse text-13' }, [
          h(
            'thead',
            h(
              'tr',
              node.head.map((cell, index) =>
                h('th', { key: index, class: 'border-b border-border px-2 py-1 text-left font-medium text-foreground' }, inline(cell)),
              ),
            ),
          ),
          h(
            'tbody',
            node.rows.map((row, rowIndex) =>
              h(
                'tr',
                { key: rowIndex },
                row.map((cell, index) =>
                  h('td', { key: index, class: 'border-b border-border px-2 py-1 align-top text-muted-foreground' }, inline(cell)),
                ),
              ),
            ),
          ),
        ]),
      ])
  }
}

/** Markdown text as readable, selectable content. */
export default defineComponent({
  name: 'MarkdownBody',
  props: { source: { type: String as PropType<string>, required: true } },
  setup(props) {
    return () => h('div', { class: 'selectable flex min-w-0 flex-col gap-3 break-words' }, parseMarkdown(props.source).map(block))
  },
})
