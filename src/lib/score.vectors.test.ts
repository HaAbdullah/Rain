// The golden vectors of the scorer. The backend ports `score.ts` to Rust (crates/rain-backend,
// modules/launcher/score.rs) and reads the same file in its own test, so neither side can change
// the scoring alone: this test fails when `score.ts` and the file disagree, and the Rust test
// fails when the port and the file disagree.
//
// The expected numbers come from `score.ts`. To write the file again after a deliberate change
// of the scorer, run `npx vitest run -u src/lib/score.vectors.test.ts` and commit the result.
import { expect, it } from 'vitest'
import { score } from './score'
import vectorsFile from './score.vectors.json?raw'

type Input = [query: string, text: string, keywords?: string[]]

/** One case for each class of the scorer and for each place where Rust and JavaScript could differ. */
const HAND_WRITTEN: Input[] = [
  // the five classes
  ['code', 'Code'],
  ['code', 'Code review notes'],
  ['code', 'CODE'],
  ['stat', 'weather-station'],
  ['stat', 'Weather station'],
  ['ion', 'weather-station'],
  ['cod', 'Visual Studio Code'],
  ['vsc', 'Visual Studio Code'],
  ['vs', 'Visual Studio Code'],
  ['sc', 'Visual Studio Code'],
  ['dc', 'Discord'],
  ['dsc', 'Discord'],
  ['dcd', 'Discord'],
  ['cr', 'Chrome'],
  ['chr', 'Chrome'],
  ['rome', 'Chrome'],
  ['hrm', 'Chrome'],
  ['x', 'Chrome'],
  ['c', 'Chrome'],
  ['e', 'Chrome'],
  // one letter
  ['a', 'a'],
  ['a', 'abc'],
  ['b', 'abc'],
  ['z', 'abc'],
  // keywords
  ['web', 'Chrome', ['browser', 'web']],
  ['web', 'Web dev', ['browser']],
  ['brow', 'Chrome', ['browser', 'web']],
  ['edit', 'Notepad', ['text', 'editor']],
  ['editor', 'Notepad', ['text', 'editor']],
  ['zzz', 'Notepad', ['text', 'editor']],
  ['te', 'Notepad', ['text', 'editor']],
  ['note', 'Notepad', ['notes']],
  ['games', 'Steam', ['games', 'store']],
  ['', 'Steam', ['games']],
  // several words
  ['dev web', 'Web dev'],
  ['web dev', 'Web dev'],
  ['dev web', 'Web development tools'],
  ['studio code', 'Visual Studio Code'],
  ['code studio', 'Visual Studio Code'],
  ['code zzz', 'Visual Studio Code'],
  ['zzz code', 'Visual Studio Code'],
  ['a b', 'Alpha beta'],
  ['b a', 'Alpha beta'],
  ['music play', 'Music player', ['audio']],
  ['audio play', 'Music player', ['audio']],
  ['dev web', 'Chrome', ['web', 'dev']],
  // empty and blank queries
  ['', 'anything'],
  ['   ', 'anything'],
  ['\t\n', 'anything'],
  ['', ''],
  // case
  ['VS', 'vscode'],
  ['vs', 'VSCODE'],
  ['CODE', 'code'],
  ['Code', 'Code'],
  // text shorter than the query, empty text
  ['abcdef', 'abc'],
  ['abc', ''],
  ['abc', 'a'],
  ['ab', 'a'],
  // long text
  ['a', 'a' + 'x'.repeat(200)],
  ['aa', 'a' + 'x'.repeat(100) + 'a'],
  ['ax', 'x'.repeat(50) + 'a' + 'x'.repeat(50)],
  ['ab', 'a' + '-'.repeat(30) + 'b'],
  // trimming and separators
  ['  code  ', 'Code'],
  ['code ', 'Code review'],
  [' co de', 'Code'],
  ['code', ' code'],
  ['code', '-code'],
  ['code', 'my-code'],
  ['code', 'my_code'],
  ['code', 'my.code'],
  ['code', 'my2code'],
  ['code', 'mycode'],
  ['code', 'my code'],
  ['code', 'a b code'],
  // a word start needs a separator that is not a-z0-9, so an accented letter counts as one
  ['code', 'écode'],
  ['code', 'xécode'],
  ['cafe', 'café cafe'],
  // Unicode lower-casing: U+0130 (dotted I), the final sigma, sharp s, a pair of surrogates
  ['i', 'İ'],
  ['i̇', 'İ'],
  ['İ', 'i̇'],
  ['σ', 'Σ'],
  ['ς', 'Σ'],
  ['οδος', 'ΟΔΟΣ'],
  ['Σ', 'σς'],
  ['ss', 'Straße'],
  ['ß', 'Straße'],
  ['stra', 'Straße'],
  ['ße', 'Straße'],
  ['\u{1F600}', 'smile \u{1F600}'],
  ['\u{1F600}', '\u{1F600}'],
  ['a', 'a\u{1F600}'],
  ['a\u{1F600}', 'a\u{1F600}b'],
  ['\u{1F600}b', 'a\u{1F600}b'],
  ['ab', 'a\u{1F600}b'],
  ['ab', 'a\u{1F600}\u{1F600}b'],
  // spaces that JavaScript knows and Rust does not, and the reverse
  [' code ', 'Code'],
  ['﻿code', 'Code'],
  ['code﻿', 'Code'],
  ['\u0085code', 'Code'],
  ['code\u0085', 'Code'],
  ['dev web', 'Web dev'],
  ['dev　web', 'Web dev'],
  ['dev\u0085web', 'Web dev'],
  ['dev﻿web', 'Web dev'],
  ['dev web', 'Web dev'],
  ['a b', 'Alpha beta'],
]

/** A small seeded generator (mulberry32), so the generated cases are the same on every run. */
function seeded(seed: number): () => number {
  let state = seed
  return () => {
    state = (state + 0x6d2b79f5) | 0
    let t = Math.imul(state ^ (state >>> 15), 1 | state)
    t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296
  }
}

const ALPHABET = ['a', 'b', 'c', 'd', 'e', '1', '2', '-', ' ', 'é', 'ß', 'İ', 'Σ', '\u{1F600}', ' ']

function generated(count: number): Input[] {
  const random = seeded(20261122)
  const pick = (length: number) =>
    Array.from({ length }, () => ALPHABET[Math.floor(random() * ALPHABET.length)]).join('')
  const cases: Input[] = []
  for (let i = 0; i < count; i += 1) {
    const text = pick(1 + Math.floor(random() * 14))
    // Half of the queries are cut out of the text, so they match; the rest are random.
    let query: string
    if (random() < 0.5) {
      const units = Array.from(text)
      const start = Math.floor(random() * units.length)
      const end = start + 1 + Math.floor(random() * (units.length - start))
      query = units.slice(start, end).join('')
    } else {
      query = pick(1 + Math.floor(random() * 4))
    }
    const keywords = random() < 0.3 ? [pick(1 + Math.floor(random() * 6))] : []
    cases.push([query, text, keywords])
  }
  return cases
}

function build(): string {
  const inputs = [...HAND_WRITTEN, ...generated(170)]
  const lines = inputs.map(([query, text, keywords = []]) => {
    const expected = score(query, text, keywords)
    return `    ${JSON.stringify({ query, text, keywords, expected })}`
  })
  const body = `{\n  "version": 1,\n  "cases": [\n${lines.join(',\n')}\n  ]\n}\n`
  // Everything outside ASCII is written as \uXXXX, so that U+00A0, U+FEFF and U+0085 are visible.
  return body.replace(/[\u007f-￿]/g, (c) => `\\u${c.charCodeAt(0).toString(16).padStart(4, '0')}`)
}

it('has the same numbers as score.ts (write the file again with `vitest -u` after a change)', async () => {
  await expect(build()).toMatchFileSnapshot('./score.vectors.json')
})

it('holds the cases that the Rust port reads', () => {
  const file = JSON.parse(vectorsFile) as { version: number; cases: { expected: number }[] }
  expect(file.version).toBe(1)
  expect(file.cases.length).toBeGreaterThanOrEqual(200)
  // The classes all occur: no match, a weak match, and an exact one.
  const numbers = new Set(file.cases.map((c) => c.expected))
  expect(numbers.has(0)).toBe(true)
  expect(numbers.has(100)).toBe(true)
})
