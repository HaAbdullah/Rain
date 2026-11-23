// Generates src/bindings/rain-types.ts from the Rust types in crates/rain-types.
//
//   node scripts/types.mjs            write the file          (npm run types)
//   node scripts/types.mjs --check    fail if it is outdated  (npm run types:check)
//
// This script exists only because `RAIN_TYPES_CHECK=1 cargo test ...` is not a command that
// cmd.exe (what npm uses on Windows) understands: here the variable is set from Node.
import { spawnSync } from 'node:child_process'

const check = process.argv.includes('--check')
const env = check ? { ...process.env, RAIN_TYPES_CHECK: '1' } : process.env

const result = spawnSync('cargo', ['test', '-p', 'rain-types', 'write_single_file'], {
  stdio: 'inherit',
  env,
})
if (result.error) {
  console.error(`could not run cargo: ${result.error.message}`)
  process.exit(1)
}
process.exit(result.status ?? 1)
