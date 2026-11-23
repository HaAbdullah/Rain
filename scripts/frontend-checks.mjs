// Two cheap checks of the frontend that no compiler makes (section 06 9.8 of docs/backend):
//   1. no sample-data code is left in the frontend;
//   2. the version is the same in package.json, src-tauri/tauri.conf.json and Cargo.toml.
//   node scripts/frontend-checks.mjs
import { existsSync, readdirSync, readFileSync, statSync } from 'node:fs'
import { join } from 'node:path'

const failures = []

/** Every file under `dir`, except the generated bindings. */
function filesUnder(dir) {
  return readdirSync(dir).flatMap((name) => {
    const path = join(dir, name)
    if (path.includes(join('src', 'bindings'))) return []
    return statSync(path).isDirectory() ? filesUnder(path) : [path]
  })
}

// ---- 1. no mock ----

const forbidden = [
  /data\/mock/,
  /mockControl/,
  /createMockApi/,
  /VITE_RAIN_MOCK/,
  /RetiredMockMethod/,
  /rain\.prototype/,
  /onReseed/,
  /from ['"][./@a-z]*\/routing['"]/,
  /Mock data|Mock scenario|Simulated latency|Reset mock|Restart prototype|reset-mock|simulate-permission/,
  /\bsetPaused\b|\bpathExists\b|\bsimulatePermission\b|scenario-/,
  // The word "prototype" as a name for the app (a `.prototype` property is fine).
  /(?<![.\w])prototype\b/i,
]
for (const file of [...filesUnder('src'), ...filesUnder('scripts'), 'package.json', 'vite.config.ts']) {
  if (file.endsWith('frontend-checks.mjs')) continue
  const text = readFileSync(file, 'utf8')
  for (const pattern of forbidden) {
    if (pattern.test(text)) failures.push(`${file}: matches ${pattern}`)
  }
}

// The built output holds no sample-data generator either (only after `npm run build`: `--dist`).
if (process.argv.includes('--dist')) {
  if (!existsSync('dist')) failures.push('dist does not exist: run npm run build first')
  else {
    for (const file of filesUnder('dist').filter((name) => /\.(js|css|html)$/.test(name))) {
      const text = readFileSync(file, 'utf8')
      for (const word of ['mulberry32', 'createMockApi', 'mockControl']) {
        if (text.includes(word)) failures.push(`${file}: contains ${word}`)
      }
    }
  }
}
for (const gone of ['src/data/mock', 'src/data/router.ts', 'src/data/routing.ts', '.env.mock']) {
  if (existsSync(gone)) failures.push(`${gone} must not exist`)
}

// types.ts is the barrel: no interface, and no type except the three aliases.
const barrel = readFileSync('src/data/types.ts', 'utf8')
const declared = [...barrel.matchAll(/^(?:export\s+)?(?:interface|type)\s+(\w+)/gm)].map((match) => match[1])
for (const name of declared) {
  if (!['ID', 'ISODate', 'ISODateTime'].includes(name)) failures.push(`src/data/types.ts declares ${name}`)
}

// ---- 2. one version ----

const versions = {
  'package.json': JSON.parse(readFileSync('package.json', 'utf8')).version,
  'src-tauri/tauri.conf.json': JSON.parse(readFileSync('src-tauri/tauri.conf.json', 'utf8')).version,
  'Cargo.toml': /^version\s*=\s*"([^"]+)"/m.exec(readFileSync('Cargo.toml', 'utf8'))?.[1],
}
if (new Set(Object.values(versions)).size !== 1) failures.push(`versions differ: ${JSON.stringify(versions)}`)

if (failures.length > 0) {
  console.error(failures.join('\n'))
  process.exit(1)
}
console.log('frontend checks: no mock code, one version.')
