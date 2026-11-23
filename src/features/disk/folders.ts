// Small helpers for walking the scanned tree by path. No d3 here, so the screen
// and its list can use them without pulling in the layout code.
import type { DiskNode } from '@/data/types'

/** True when `path` is `ancestor` itself or lies below it. Windows paths: the root is "C:\". */
export function isInside(path: string, ancestor: string): boolean {
  const prefix = ancestor.endsWith('\\') ? ancestor : `${ancestor}\\`
  return path === ancestor || path.startsWith(prefix)
}

/** The nodes from the root down to `path`. Stops at the deepest node found if the path is unknown. */
export function trailTo(root: DiskNode, path: string): DiskNode[] {
  const trail = [root]
  let node = root
  while (node.path !== path) {
    const next = node.children?.find((child) => child.kind === 'dir' && isInside(path, child.path))
    if (!next) break
    trail.push(next)
    node = next
  }
  return trail
}

/** A node's share of its parent, from 0 to 1. */
export function shareOf(node: DiskNode, parent: DiskNode): number {
  return parent.sizeBytes > 0 ? node.sizeBytes / parent.sizeBytes : 0
}

/** True when opening `node` needs a fetch: its children are missing, or only the largest are there. */
export function needsLoad(node: DiskNode, loaded: ReadonlySet<string>): boolean {
  if (node.kind !== 'dir' || node.denied || loaded.has(node.path)) return false
  return node.truncated === true || (node.children?.some((child) => child.kind === 'other') ?? false)
}

/** A copy of `root` in which the folder with the path of `replacement` is replaced by it. Shares everything else. */
export function replaceNode(root: DiskNode, replacement: DiskNode): DiskNode {
  if (root.path === replacement.path) return replacement
  if (!root.children || !isInside(replacement.path, root.path)) return root
  const index = root.children.findIndex((child) => child.kind === 'dir' && isInside(replacement.path, child.path))
  if (index < 0) return root
  const replaced = replaceNode(root.children[index], replacement)
  if (replaced === root.children[index]) return root
  const children = root.children.slice()
  children[index] = replaced
  return { ...root, children }
}
