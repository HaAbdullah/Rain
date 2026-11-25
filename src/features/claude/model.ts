// What the Claude screen shows, as plain functions: which files belong to a sub-view, how a list is
// filtered and grouped, and the words and icons of each kind. No Vue here, so it is easy to test.
import type { SelGroup } from '@/composables/useSelection'
import type { LibraryItem, LibraryKind, LibrarySetup } from '@/data/types'
import { matches } from '@/lib/score'

export const SUB_VIEWS = ['skills', 'memory', 'instructions', 'agents', 'setup'] as const
export type SubView = (typeof SUB_VIEWS)[number]
/** The sub-views that list files. */
export type ListView = Exclude<SubView, 'setup'>

/** Which kinds of file each list sub-view shows. Agents and commands share one list. */
export const KINDS_OF_VIEW: Record<ListView, readonly LibraryKind[]> = {
  skills: ['skill'],
  memory: ['memory'],
  instructions: ['instruction'],
  agents: ['agent', 'command'],
}

export const KIND_LABEL: Record<LibraryKind, string> = {
  skill: 'Skill',
  memory: 'Memory',
  instruction: 'Instructions',
  agent: 'Agent',
  command: 'Command',
}

export const KIND_ICON: Record<LibraryKind, string> = {
  skill: 'sparkles',
  memory: 'brain',
  instruction: 'scroll-text',
  agent: 'bot',
  command: 'square-terminal',
}

export const VIEW_ICON: Record<SubView, string> = {
  skills: 'sparkles',
  memory: 'brain',
  instructions: 'scroll-text',
  agents: 'bot',
  setup: 'wrench',
}

export const VIEW_LABEL: Record<SubView, string> = {
  skills: 'Skills',
  memory: 'Memory',
  instructions: 'Instructions',
  agents: 'Agents',
  setup: 'Setup',
}

/** What the empty list of each sub-view says. */
export const EMPTY_TEXT: Record<ListView, { title: string; hint: string }> = {
  skills: { title: 'No skills', hint: 'Skills are folders with a SKILL.md in your Claude folder, in a plugin or in a project' },
  memory: { title: 'No memory files', hint: 'Claude keeps one memory folder for each project it worked in' },
  instructions: { title: 'No instructions', hint: 'Instructions are CLAUDE.md files in your Claude folder or in a project' },
  agents: { title: 'No agents or commands', hint: 'They are markdown files in the agents and commands folders of Claude' },
}

export function itemsOfView(items: readonly LibraryItem[], view: ListView): LibraryItem[] {
  const kinds = KINDS_OF_VIEW[view]
  return items.filter((item) => kinds.includes(item.kind))
}

/** How many files each list sub-view has, for the strip. */
export function countsByView(items: readonly LibraryItem[]): Record<ListView, number> {
  return {
    skills: itemsOfView(items, 'skills').length,
    memory: itemsOfView(items, 'memory').length,
    instructions: itemsOfView(items, 'instructions').length,
    agents: itemsOfView(items, 'agents').length,
  }
}

/** The number of facts the Setup sub-view lists. */
export function setupCount(setup: LibrarySetup): number {
  return setup.plugins.length + setup.mcpServers.length + setup.hooks.length + setup.outputStyles.length
}

export function matchesItem(query: string, item: LibraryItem): boolean {
  return matches(query, item.name, [item.description, item.scope, item.memoryType ?? ''])
}

/**
 * The list of one sub-view, filtered by the query and grouped by scope. The backend sends the
 * files in the order that the list shows (you, projects, plugins), and the groups keep it.
 * A memory file called MEMORY is the index of its project and goes first in its group.
 */
export function groupsOf(items: readonly LibraryItem[], view: ListView, query: string): SelGroup<LibraryItem>[] {
  const groups = new Map<string, LibraryItem[]>()
  for (const item of itemsOfView(items, view)) {
    if (!matchesItem(query, item)) continue
    const list = groups.get(item.scope)
    if (list) list.push(item)
    else groups.set(item.scope, [item])
  }
  return [...groups].map(([scope, list]) => ({
    id: scope,
    title: scope,
    count: list.length,
    items: view === 'memory' ? [...list].sort((a, b) => Number(b.name === 'MEMORY') - Number(a.name === 'MEMORY')) : list,
  }))
}

/** Front matter keys read like words: `node_type` is "node type", `metadata.type` is "type". */
export function factLabel(key: string): string {
  const last = key.includes('.') ? key.slice(key.lastIndexOf('.') + 1) : key
  return last.replace(/[_-]+/g, ' ')
}
