// One place that turns anything a call can throw into a RainError (foundation 8.3).
// The bridges already reject with the backend's error body or one of the same shape;
// this is the net for everything else (a bug, a string from the shell, a failed import).
import type { RainError } from '../api'
import { errorMessage } from '@/lib/errors'

function looksLikeRainError(reason: unknown): reason is RainError {
  if (typeof reason !== 'object' || reason === null) return false
  const { code, message } = reason as Partial<Record<keyof RainError, unknown>>
  return typeof code === 'string' && typeof message === 'string'
}

/** An error with `code` and `message` passes through unchanged; anything else becomes `failed`. */
export function toRainError(reason: unknown): RainError {
  if (looksLikeRainError(reason)) return reason
  return { code: 'failed', message: errorMessage(reason, String(reason)) }
}
