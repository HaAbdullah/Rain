// Text matching for the launcher (ranked `score`) and for the section filter
// (plain `matches`). Both ignore case. No fuzzy-search library is used.

// One base value per match class. The gaps leave room for the closeness bonus
// (0..9), so a better class always wins over a better bonus.
const EXACT = 100
const PREFIX = 80
const WORD_START = 60
const SUBSTRING = 40
const SUBSEQUENCE = 20

/** A keyword hit ranks just under the same kind of hit on the visible text. */
const KEYWORD_PENALTY = 5

/** True when `q` starts a word in `t` ("stat" in "weather-station"). */
function startsWord(t: string, q: string): boolean {
  let at = t.indexOf(q, 1)
  while (at !== -1) {
    if (!/[a-z0-9]/.test(t[at - 1])) return true
    at = t.indexOf(q, at + 1)
  }
  return false
}

/** Length of the shortest run of `t` that holds the letters of `q` in order, or 0. */
function subsequenceSpan(t: string, q: string): number {
  let best = 0
  for (let start = t.indexOf(q[0]); start !== -1; start = t.indexOf(q[0], start + 1)) {
    let ti = start
    let qi = 0
    while (ti < t.length && qi < q.length) {
      if (t[ti] === q[qi]) qi += 1
      ti += 1
    }
    if (qi < q.length) break
    const span = ti - start
    if (best === 0 || span < best) best = span
  }
  return best
}

/** Scores one lower-cased text against one lower-cased, non-empty query. */
function scoreText(q: string, t: string): number {
  if (t.length === 0) return 0
  if (t === q) return EXACT

  // A shorter text is a closer match: "code" should rank "Code" above "Code review notes".
  const closeness = Math.round((9 * q.length) / t.length)

  if (t.startsWith(q)) return PREFIX + closeness
  if (startsWord(t, q)) return WORD_START + closeness
  if (t.includes(q)) return SUBSTRING + closeness

  // One letter "in order" matches nearly everything, so it is not worth a result.
  if (q.length < 2) return 0
  const span = subsequenceSpan(t, q)
  return span === 0 ? 0 : SUBSEQUENCE + Math.round((9 * q.length) / span)
}

function scoreOne(q: string, text: string, keywords: readonly string[]): number {
  let best = scoreText(q, text.toLowerCase())
  for (const keyword of keywords) {
    const s = scoreText(q, keyword.toLowerCase())
    if (s > 0) best = Math.max(best, s - KEYWORD_PENALTY)
  }
  return best
}

/**
 * Rank of `text` for `query`: 0 = no match, higher is better
 * (exact > prefix > word start > substring > letters in order).
 * An empty query matches everything equally and returns 1.
 */
export function score(query: string, text: string, keywords: readonly string[] = []): number {
  const q = query.trim().toLowerCase()
  if (q === '') return 1

  const whole = scoreOne(q, text, keywords)
  if (whole > 0) return whole

  // "dev web" should still find "Web dev": every word must match on its own.
  // Such a result ranks below any match of the whole query.
  const words = q.split(/\s+/)
  if (words.length < 2) return 0
  let weakest = Infinity
  for (const word of words) {
    const s = scoreOne(word, text, keywords)
    if (s === 0) return 0
    weakest = Math.min(weakest, s)
  }
  return Math.max(1, Math.round(weakest / 10))
}

/** Section filter: case-insensitive substring on the text and the keywords. An empty query matches. */
export function matches(query: string, text: string, keywords: readonly string[] = []): boolean {
  const q = query.trim().toLowerCase()
  if (q === '') return true
  if (text.toLowerCase().includes(q)) return true
  return keywords.some((keyword) => keyword.toLowerCase().includes(q))
}
