// The notice that follows an answer to a permission request. Pure, so a test can read every wording.
import type { PermissionAnswer, PermissionAnswerResult } from '@/data/types'

/**
 * `result.applied` is what really happened: "Always allow" becomes "once" when no session rule
 * could be made, and the notice then says so instead of claiming a rule that does not exist.
 * `asked` is what the user pressed (the result alone cannot tell a plain "once" from that case).
 * `waitingLeft` is how many other sessions still wait on the user.
 */
export function answerNotice(
  result: PermissionAnswerResult,
  waitingLeft: number,
  asked: PermissionAnswer = result.applied,
): string {
  const text = headline(result, asked)
  return waitingLeft > 0 ? `${text}, ${waitingLeft} waiting` : text
}

function headline({ applied, ruleText }: PermissionAnswerResult, asked: PermissionAnswer): string {
  switch (applied) {
    case 'deny':
      return 'Denied'
    case 'always':
      return ruleText ? `Always allowed ${ruleText} for this session` : 'Always allowed for this session'
    case 'once':
      return asked === 'always' ? 'Allowed once. No session rule was possible.' : 'Allowed'
  }
}
