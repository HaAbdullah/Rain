import { describe, expect, it } from 'vitest'
import type { LibraryItem, LibraryKind } from '@/data/types'
import { countsByView, factLabel, groupsOf, itemsOfView, matchesItem } from './model'

function item(name: string, kind: LibraryKind, scope = 'You', description = ''): LibraryItem {
  return { id: `cl-${name}-${scope}`, kind, name, description, scope, path: `C:\\x\\${name}.md`, size: 1 }
}

const ITEMS = [
  item('alpha', 'skill'),
  item('beta', 'skill', 'Plugin tools', 'Builds the beta thing'),
  item('helper', 'agent'),
  item('ship', 'command', 'Project Shop'),
  item('note', 'memory', 'Project Shop'),
  item('MEMORY', 'memory', 'Project Shop'),
  item('CLAUDE.md', 'instruction'),
]

describe('the sub-views', () => {
  it('put agents and commands in one list', () => {
    expect(itemsOfView(ITEMS, 'agents').map((entry) => entry.name)).toEqual(['helper', 'ship'])
  })

  it('count the files of each list', () => {
    expect(countsByView(ITEMS)).toEqual({ skills: 2, memory: 2, instructions: 1, agents: 2 })
  })
})

describe('the list of a sub-view', () => {
  it('is grouped by scope in the order it came', () => {
    const groups = groupsOf(ITEMS, 'skills', '')
    expect(groups.map((group) => [group.title, group.items.length])).toEqual([
      ['You', 1],
      ['Plugin tools', 1],
    ])
  })

  it('is filtered by name, description and scope', () => {
    expect(matchesItem('beta', ITEMS[1])).toBe(true)
    expect(matchesItem('builds', ITEMS[1])).toBe(true)
    expect(matchesItem('plugin', ITEMS[1])).toBe(true)
    expect(matchesItem('zzz', ITEMS[1])).toBe(false)
    expect(groupsOf(ITEMS, 'skills', 'builds').flatMap((group) => group.items)).toHaveLength(1)
    expect(groupsOf(ITEMS, 'skills', 'zzz')).toEqual([])
  })

  it('puts the memory index first in its project', () => {
    const [group] = groupsOf(ITEMS, 'memory', '')
    expect(group.items.map((entry) => entry.name)).toEqual(['MEMORY', 'note'])
  })
})

describe('fact labels', () => {
  it('read like words', () => {
    expect(factLabel('metadata.node_type')).toBe('node type')
    expect(factLabel('allowed-tools')).toBe('allowed tools')
    expect(factLabel('name')).toBe('name')
  })
})
