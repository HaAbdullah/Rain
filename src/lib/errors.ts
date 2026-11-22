// One way to read the message of a rejection. Anything can be thrown, so the
// reason is checked before use; a null or message-less reason gives `fallback`.
export function errorMessage(reason: unknown, fallback: string): string {
  if (typeof reason === 'object' && reason !== null && 'message' in reason) {
    const { message } = reason
    if (typeof message === 'string' && message !== '') return message
  }
  return fallback
}
