// The app's tsconfig has no Node types. The theme tests read tokens.css and index.html as
// text (a CSS import is empty under Vitest), which needs this one function of node:fs.
declare module 'node:fs' {
  export function readFileSync(path: string, encoding: 'utf8'): string
}
