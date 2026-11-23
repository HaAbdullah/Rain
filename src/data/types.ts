// Every entity the UI reads. The shapes are generated from the Rust crate rain-types
// (src/bindings/rain-types.ts, `npm run types`) and re-exported here, so screens keep
// importing from this one file. Nothing is declared by hand except the three aliases below
// (scripts/barrel.mjs fails on a name that is in both places).

export * from '../bindings/rain-types'

// Hand-written aliases: the generator turns these into plain `string`.
export type ID = string
export type ISODate = string        // '2026-11-21'
export type ISODateTime = string    // '2026-11-21T09:30:00.000Z'
