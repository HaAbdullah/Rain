// The routing table of the current stage. A stage that moves a method to the backend edits
// REAL_METHODS and REAL_TOPICS here in the same commit, so the move is a visible decision.
import { describe, expect, it } from 'vitest'
import type { Routing } from './routing'
import { routing } from './routing'

// Stage 1b: settings (calls and topic) and the health topic. Stage 2: getHealth.
// A section adds its names under its own comment, one per line, and moves nothing else.
const REAL_METHODS = [
  'getSettings',
  'updateSettings',
  'getHealth',
  // launcher and apps (section 01)
  // projects and games (section 02)
  // sessions and jobs (section 03)
  // life (section 04)
  // disk and machine (section 05)
  // game mode (section 06)
]
const REAL_TOPICS = [
  'settings',
  'health',
  // one line per topic that becomes real
]

describe('routing table', () => {
  it('sends exactly the methods of the finished stages to the backend', () => {
    const real = Object.entries(routing.methods)
      .filter(([, side]) => side === 'real')
      .map(([name]) => name)
    expect(real.sort()).toEqual([...REAL_METHODS].sort())
  })

  it('sends exactly the topics of the finished stages to the backend', () => {
    const real = Object.entries(routing.topics)
      .filter(([, side]) => side === 'real')
      .map(([name]) => name)
    expect(real.sort()).toEqual([...REAL_TOPICS].sort())
  })

  it('names only two sides', () => {
    const sides = new Set([...Object.values(routing.methods), ...Object.values(routing.topics)])
    expect([...sides].sort()).toEqual(['mock', 'real'])
  })
})

describe('routing table: retired mock methods', () => {
  it('does not let a method the mock has retired be routed to the mock', () => {
    // A compile-time check: `vue-tsc` fails when the directive below is not needed.
    // @ts-expect-error getHealth is in RetiredMockMethod, so only 'real' is allowed
    const refused: Routing['methods']['getHealth'] = 'mock'
    expect(refused).toBe('mock')
    expect(routing.methods.getHealth).toBe('real')
  })
})
