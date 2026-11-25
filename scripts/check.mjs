// The checklist every stage must pass (foundation section 12.4). Runs the checks in order and
// stops at the first one that fails.   npm run check
//
//   npm run check:quick    the half-minute version for the edit loop: it compiles (Rust and
//                          TypeScript) and runs the frontend tests. No lints, no Rust tests,
//                          no build, no start of the app.
import { spawnSync } from 'node:child_process'

const quickChecks = [
  ['cargo check --workspace --all-targets', 'the Rust code compiles'],
  ['npm run typecheck', 'the TypeScript code compiles'],
  ['npm test', 'frontend tests'],
]

const fullChecks = [
  ['cargo fmt --all --check', 'formatting'],
  ['cargo build --workspace', 'everything compiles'],
  ['cargo clippy --workspace --all-targets -- -D warnings', 'lints'],
  // Before the tests: the test run rewrites the generated file, which would hide a drift.
  ['npm run types:check', 'generated types are current'],
  ['npm run types:barrel', 'no type is declared in both places'],
  ['cargo test --workspace', 'unit and integration tests'],
  ['node scripts/frontend-checks.mjs', 'no mock code is left, one version'],
  ['npm test', 'frontend tests'],
  ['npm run build', 'vue-tsc and vite build'],
  ['node scripts/frontend-checks.mjs --dist', 'the built output holds no mock code'],
  ['npm run smoke', 'the app starts'],
]

const checks = process.argv.includes('--quick') ? quickChecks : fullChecks

const started = Date.now()
for (const [index, [command, meaning]] of checks.entries()) {
  console.log(`\n[${index + 1}/${checks.length}] ${command}  (${meaning})`)
  // `shell: true` because npm is a .cmd file on Windows and cannot be started directly.
  const result = spawnSync(command, { stdio: 'inherit', shell: true })
  if (result.status !== 0) {
    console.error(`\nCHECK FAILED: ${command}`)
    process.exit(result.status ?? 1)
  }
}
console.log(`\nAll checks passed in ${Math.round((Date.now() - started) / 1000)} s.`)
