// Fails when a name is exported by the generated types and also declared by hand in
// src/data/types.ts (foundation section 8.8).   npm run types:barrel
//
// TypeScript lets a local export silently win over one that comes from `export *`, so a type
// that was ported to Rust but not deleted from types.ts would not be an error by itself.
import { readFileSync } from 'node:fs'

const GENERATED = 'src/bindings/rain-types.ts'
const BARREL = 'src/data/types.ts'

/** The names a file declares with `export type|interface|const|enum|function|class <Name>`. */
function declaredNames(file) {
  const pattern = /^export\s+(?:declare\s+)?(?:type|interface|const|enum|function|class)\s+([A-Za-z_$][\w$]*)/gm
  return [...readFileSync(file, 'utf8').matchAll(pattern)].map((match) => match[1])
}

const generated = new Set(declaredNames(GENERATED))
const duplicates = declaredNames(BARREL).filter((name) => generated.has(name))

if (duplicates.length > 0) {
  console.error(
    `${BARREL} declares ${duplicates.join(', ')}, which ${GENERATED} already exports.\n` +
      'A stage that ports a type must delete the hand-written one in the same change.',
  )
  process.exit(1)
}
console.log(`types barrel: no name is declared twice (${generated.size} generated names).`)
