// The treemap: layout with d3-hierarchy, drawing on a 2D canvas, and hit-testing.
// d3-hierarchy is imported only here (spec 7.7).
import { hierarchy, treemap, treemapSquarify, type HierarchyRectangularNode } from 'd3-hierarchy'
import type { DiskNode, FileCategory } from '@/data/types'

export type TreemapRect = HierarchyRectangularNode<DiskNode>

export const CATEGORIES: readonly { id: FileCategory; label: string }[] = [
  { id: 'video', label: 'Video' },
  { id: 'image', label: 'Images' },
  { id: 'audio', label: 'Audio' },
  { id: 'archive', label: 'Archives' },
  { id: 'code', label: 'Code' },
  { id: 'document', label: 'Documents' },
  { id: 'app', label: 'Apps' },
  { id: 'other', label: 'Other' },
]

/** Levels drawn below the current folder; deeper folders are drawn as one block. */
const DEPTH_CAP = 4
const PADDING = 1
/** Rectangles with a side under this are neither drawn nor hit. */
const MIN_SIDE = 2
const LABEL_MIN_WIDTH = 80
const LABEL_MIN_HEIGHT = 24
const LABEL_INSET = 6
/** Fills are drawn fully opaque: the label colour (--on-file) is checked against the plain fill, in every theme. */
const FILL_ALPHA = 1

export interface Palette {
  fills: Record<FileCategory, string>
  background: string
  frame: string
  text: string
  outline: string
  font: string
}

/** Reads the colour tokens once, because a canvas cannot use CSS variables. */
export function readPalette(): Palette {
  const style = getComputedStyle(document.documentElement)
  const token = (name: string) => style.getPropertyValue(name).trim()
  return {
    fills: {
      video: token('--file-video'),
      image: token('--file-image'),
      audio: token('--file-audio'),
      archive: token('--file-archive'),
      code: token('--file-code'),
      document: token('--file-document'),
      app: token('--file-app'),
      other: token('--file-other'),
    },
    background: token('--surface-0'),
    frame: token('--border-strong'),
    text: token('--on-file'),
    outline: token('--foreground'),
    font: `12px ${token('--rain-font-sans')}`,
  }
}

/** Lays `folder` out in a `width` x `height` box. The result's root is the folder itself. */
export function layoutTreemap(folder: DiskNode, width: number, height: number): TreemapRect {
  const root = hierarchy(folder, (node) => node.children)
  const cutOff = new Set<DiskNode>()
  root.each((node) => {
    if (node.depth < DEPTH_CAP) return
    if (node.children) cutOff.add(node.data)
    node.children = undefined
  })
  // A folder cut off at the cap counts with its whole size; one with children counts through them.
  root.sum((node) => (node.children && !cutOff.has(node) ? 0 : node.sizeBytes))
  root.sort((a, b) => (b.value ?? 0) - (a.value ?? 0))
  return treemap<DiskNode>().tile(treemapSquarify).size([width, height]).padding(PADDING).round(true)(root)
}

const dominantCache = new WeakMap<DiskNode, FileCategory>()

/** The category that takes the most bytes inside a folder; colours a folder that is drawn as one block. */
function dominantCategory(folder: DiskNode): FileCategory {
  const cached = dominantCache.get(folder)
  if (cached) return cached

  const totals = new Map<FileCategory, number>()
  const walk = (node: DiskNode) => {
    if (node.children) {
      node.children.forEach(walk)
      return
    }
    const category = node.category ?? 'other'
    totals.set(category, (totals.get(category) ?? 0) + node.sizeBytes)
  }
  walk(folder)

  let best: FileCategory = 'other'
  let bestBytes = -1
  for (const [category, bytes] of totals) {
    if (bytes > bestBytes) {
      best = category
      bestBytes = bytes
    }
  }
  dominantCache.set(folder, best)
  return best
}

function categoryOf(node: DiskNode): FileCategory {
  return node.kind === 'dir' ? dominantCategory(node) : (node.category ?? 'other')
}

function isDrawn(rect: TreemapRect): boolean {
  return rect.x1 - rect.x0 >= MIN_SIDE && rect.y1 - rect.y0 >= MIN_SIDE
}

/** `text` cut with an ellipsis so it fits in `maxWidth`. */
function fitText(ctx: CanvasRenderingContext2D, text: string, maxWidth: number): string {
  if (ctx.measureText(text).width <= maxWidth) return text
  let end = text.length
  while (end > 1 && ctx.measureText(`${text.slice(0, end)}…`).width > maxWidth) end -= 1
  return `${text.slice(0, end)}…`
}

/** The rectangle of a direct child of the laid-out folder, by path. */
export function topLevelRect(root: TreemapRect, path: string | null): TreemapRect | undefined {
  return root.children?.find((child) => child.data.path === path)
}

/** Draws the whole map. `selectedPath` is a direct child of the folder, or null. */
export function drawTreemap(
  ctx: CanvasRenderingContext2D,
  root: TreemapRect,
  palette: Palette,
  selectedPath: string | null,
): void {
  ctx.fillStyle = palette.background
  ctx.fillRect(0, 0, root.x1, root.y1)

  ctx.font = palette.font
  ctx.textBaseline = 'middle'
  ctx.lineWidth = 1
  // each() visits parents before children, so a folder's frame lies under its contents.
  root.each((rect) => {
    if (rect === root || !isDrawn(rect)) return
    const width = rect.x1 - rect.x0
    const height = rect.y1 - rect.y0

    if (rect.children) {
      ctx.strokeStyle = palette.frame
      ctx.strokeRect(rect.x0 + 0.5, rect.y0 + 0.5, width - 1, height - 1)
      return
    }

    ctx.globalAlpha = FILL_ALPHA
    ctx.fillStyle = palette.fills[categoryOf(rect.data)]
    ctx.fillRect(rect.x0, rect.y0, width, height)
    ctx.globalAlpha = 1

    if (width >= LABEL_MIN_WIDTH && height >= LABEL_MIN_HEIGHT) {
      ctx.fillStyle = palette.text
      ctx.fillText(
        fitText(ctx, rect.data.name, width - LABEL_INSET * 2),
        rect.x0 + LABEL_INSET,
        rect.y0 + LABEL_MIN_HEIGHT / 2,
      )
    }
  })

  const selected = topLevelRect(root, selectedPath)
  if (selected && isDrawn(selected)) {
    ctx.strokeStyle = palette.outline
    ctx.lineWidth = 2
    ctx.strokeRect(selected.x0 + 1, selected.y0 + 1, selected.x1 - selected.x0 - 2, selected.y1 - selected.y0 - 2)
  }
}

/**
 * The direct child of the folder (a row of the list) under the pointer, or null.
 * It walks down the hierarchy from the pointer, so no rectangle needs a listener of its own.
 */
export function hitTest(root: TreemapRect, x: number, y: number): TreemapRect | null {
  let rect = root
  let topLevel: TreemapRect | null = null
  while (rect.children) {
    const inside = rect.children.find(
      (child) => isDrawn(child) && x >= child.x0 && x < child.x1 && y >= child.y0 && y < child.y1,
    )
    if (!inside) break
    topLevel ??= inside
    rect = inside
  }
  return topLevel
}
