import { describe, expect, it } from 'vitest'
import type { DiskNode } from '@/data/types'
import { needsLoad, replaceNode } from './folders'

const dir = (path: string, over: Partial<DiskNode> = {}): DiskNode => ({
  name: path.split('\\').filter(Boolean).at(-1) ?? path,
  path,
  sizeBytes: 100,
  kind: 'dir',
  ...over,
})
const file = (path: string): DiskNode => ({ name: 'f', path, sizeBytes: 1, kind: 'file', category: 'other' })
const other = (path: string): DiskNode => ({ name: 'other (3 items)', path, sizeBytes: 3, kind: 'other' })

describe('needsLoad', () => {
  const none = new Set<string>()

  it('is false for a file, a denied folder, a loaded folder and a complete folder', () => {
    expect(needsLoad(file('C:\\a'), none)).toBe(false)
    expect(needsLoad(dir('C:\\a', { denied: true }), none)).toBe(false)
    expect(needsLoad(dir('C:\\a', { truncated: true }), new Set(['C:\\a']))).toBe(false)
    expect(needsLoad(dir('C:\\a', { children: [file('C:\\a\\x')] }), none)).toBe(false)
    expect(needsLoad(dir('C:\\a'), none)).toBe(false)
  })

  it('is true for a truncated folder and for a folder with an other child', () => {
    expect(needsLoad(dir('C:\\a', { truncated: true }), none)).toBe(true)
    expect(needsLoad(dir('C:\\a', { children: [file('C:\\a\\x'), other('C:\\a\\*')] }), none)).toBe(true)
  })
})

describe('replaceNode', () => {
  const sibling = dir('C:\\b', { children: [file('C:\\b\\x')] })
  const deep = dir('C:\\a\\c', { truncated: true })
  const middle = dir('C:\\a', { children: [deep] })
  const root = dir('C:\\', { children: [middle, sibling] })

  it('replaces a deep folder and shares the untouched siblings', () => {
    const whole = dir('C:\\a\\c', { children: [file('C:\\a\\c\\y')] })
    const result = replaceNode(root, whole)
    expect(result).not.toBe(root)
    expect(result.children?.[0].children?.[0]).toBe(whole)
    expect(result.children?.[1]).toBe(sibling)
    expect(root.children?.[0].children?.[0]).toBe(deep) // the old tree is not changed
  })

  it('replaces the root itself', () => {
    const whole = dir('C:\\', { children: [] })
    expect(replaceNode(root, whole)).toBe(whole)
  })

  it('returns the same root when the path is not found', () => {
    expect(replaceNode(root, dir('C:\\zzz', { children: [] }))).toBe(root)
    expect(replaceNode(root, dir('D:\\a', { children: [] }))).toBe(root)
    expect(replaceNode(root, dir('C:\\a\\nope', { children: [] }))).toBe(root)
  })
})
