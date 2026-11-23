import { describe, expect, it } from 'vitest'
import { configIssueText, monitorClaudeRowState } from './rowState'

describe('monitorClaudeRowState', () => {
  it('shows nothing when the switch is off, so the row keeps its plain hint', () => {
    expect(monitorClaudeRowState('off')).toBeUndefined()
  })

  it('shows the state the backend reports', () => {
    expect(monitorClaudeRowState('on')).toEqual({ status: 'success', text: 'Active' })
    expect(monitorClaudeRowState('onInactive')).toEqual({
      status: 'idle',
      text: 'Installed. Waiting for a session to start',
    })
    expect(monitorClaudeRowState('needsRepair')).toEqual({
      status: 'failed',
      text: 'Needs repair. Press Ctrl+Enter',
    })
  })
})

describe('configIssueText', () => {
  it('names the file and the line', () => {
    expect(configIssueText({ file: 'settings.toml', line: 4, message: 'expected a value' })).toBe(
      'settings.toml has an error on line 4',
    )
  })

  it('names only the file when the parser gave no line', () => {
    expect(configIssueText({ file: 'machine.toml', message: 'unreadable' })).toBe('machine.toml has an error')
  })
})
