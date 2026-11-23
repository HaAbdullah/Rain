import { describe, expect, it } from 'vitest'
import { answerNotice } from './answerText'

describe('answerNotice', () => {
  it('says Allowed for once and Denied for deny', () => {
    expect(answerNotice({ applied: 'once' }, 0)).toBe('Allowed')
    expect(answerNotice({ applied: 'deny' }, 0)).toBe('Denied')
  })

  it('names the rule that holds for the session when "always" worked', () => {
    expect(answerNotice({ applied: 'always', ruleText: 'Bash(npm install)' }, 0, 'always')).toBe(
      'Always allowed Bash(npm install) for this session',
    )
    expect(answerNotice({ applied: 'always' }, 0, 'always')).toBe('Always allowed for this session')
  })

  it('does not claim a rule when "always" was applied as once', () => {
    expect(answerNotice({ applied: 'once' }, 0, 'always')).toBe('Allowed once. No session rule was possible.')
  })

  it('adds how many sessions still wait', () => {
    expect(answerNotice({ applied: 'once' }, 1)).toBe('Allowed, 1 waiting')
    expect(answerNotice({ applied: 'deny' }, 3, 'deny')).toBe('Denied, 3 waiting')
  })
})
