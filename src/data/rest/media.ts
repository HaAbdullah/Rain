// Images are not part of the JSON. The backend sends a reference such as
// `media/app-icon/app-vscode?v=3fa9` and the page needs an address it can load, which
// differs between the Tauri window and a browser tab (foundation 4.7 and 8.4). Rewriting
// every answer here means no screen has to know where images come from.

const MEDIA_PREFIX = 'media/'

type Json = Record<string, unknown> | unknown[]

function isContainer(value: unknown): value is Json {
  return typeof value === 'object' && value !== null
}

function isMediaRef(value: unknown): value is string {
  return typeof value === 'string' && value.startsWith(MEDIA_PREFIX)
}

/**
 * Replaces the media references in a parsed answer with `toUrl(reference)`. A reference is a
 * string that starts with `media/` and sits in a field named `coverUrl`, or in `value` of an
 * object whose `kind` is `'url'`. The answer is changed in place (it was just parsed, nobody
 * else holds it) and returned. It is walked with a stack, so a deep tree cannot overflow.
 */
export function resolveMedia<T>(answer: T, toUrl: (reference: string) => string): T {
  const pending: unknown[] = [answer]
  while (pending.length > 0) {
    const node = pending.pop()
    if (!isContainer(node)) continue

    if (Array.isArray(node)) {
      for (const item of node) pending.push(item)
      continue
    }
    for (const [key, value] of Object.entries(node)) {
      const isMediaField = key === 'coverUrl' || (key === 'value' && node.kind === 'url')
      if (isMediaField && isMediaRef(value)) node[key] = toUrl(value)
      else pending.push(value)
    }
  }
  return answer
}
