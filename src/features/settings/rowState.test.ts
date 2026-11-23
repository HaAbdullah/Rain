import { describe, expect, it } from 'vitest'
import { configIssueText, monitorClaudeRowState, sourceRowState } from './rowState'

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

describe('sourceRowState', () => {
  it('says nothing for a source that is fine, missing or silent', () => {
    expect(sourceRowState(undefined)).toBeUndefined()
    expect(sourceRowState({ name: 'steam', state: 'ok' })).toBeUndefined()
    expect(sourceRowState({ name: 'steam', state: 'off' })).toBeUndefined()
  })

  it('shows the backend text, failed or quiet', () => {
    expect(sourceRowState({ name: 'steam', state: 'failed', message: 'Cannot read libraryfolders.vdf' })).toEqual({
      status: 'failed',
      text: 'Cannot read libraryfolders.vdf',
    })
    expect(sourceRowState({ name: 'epic', state: 'off', message: 'Epic is not installed' })).toEqual({
      status: 'idle',
      text: 'Epic is not installed',
    })
  })
})
