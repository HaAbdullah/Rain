// Generated game covers: calm abstract SVG pictures (2:3) as data URIs, drawn
// from a hash of the title. No real artwork is used anywhere in the prototype.

import { createRng } from './engine'
import type { Rng } from './engine'

const WIDTH = 300
const HEIGHT = 450

interface Palette { hue: number; accentHue: number }

// Low saturation and low lightness so a wall of covers sits quietly on the dark UI.
function tone(hue: number, lightness: number, saturation = 28): string {
  return `hsl(${Math.round(hue)} ${saturation}% ${lightness}%)`
}

function orbs(rng: Rng, p: Palette): string {
  let shapes = ''
  for (let i = 0; i < 4; i += 1) {
    const radius = rng.int(50, 150)
    shapes += `<circle cx="${rng.int(0, WIDTH)}" cy="${rng.int(40, HEIGHT - 40)}" r="${radius}" `
      + `fill="${tone(i % 2 ? p.accentHue : p.hue, rng.int(30, 46))}" opacity="0.55"/>`
  }
  return shapes
}

function bands(rng: Rng, p: Palette): string {
  let shapes = ''
  const tilt = rng.int(-40, 40)
  for (let i = 0; i < 5; i += 1) {
    const y = 60 + i * 80 + rng.int(-15, 15)
    shapes += `<path d="M0 ${y} L${WIDTH} ${y + tilt} L${WIDTH} ${y + tilt + 44} L0 ${y + 44} Z" `
      + `fill="${tone(i % 2 ? p.hue : p.accentHue, 28 + i * 4)}" opacity="0.6"/>`
  }
  return shapes
}

function peaks(rng: Rng, p: Palette): string {
  let shapes = `<circle cx="${rng.int(60, 240)}" cy="${rng.int(90, 170)}" r="${rng.int(26, 44)}" `
    + `fill="${tone(p.accentHue, 52, 34)}" opacity="0.7"/>`
  for (let layer = 0; layer < 3; layer += 1) {
    const base = 260 + layer * 60
    let d = `M0 ${HEIGHT} L0 ${base}`
    for (let x = 0; x <= WIDTH; x += 50) d += ` L${x} ${base - rng.int(0, 90)}`
    d += ` L${WIDTH} ${HEIGHT} Z`
    shapes += `<path d="${d}" fill="${tone(p.hue, 34 - layer * 8)}"/>`
  }
  return shapes
}

const MOTIFS = [orbs, bands, peaks]

/** A cover for a game title. The same title always gives the same picture. */
export function coverFor(title: string): string {
  const rng = createRng(`cover:${title}`)
  const hue = rng.int(0, 359)
  const palette: Palette = { hue, accentHue: (hue + rng.int(25, 70)) % 360 }
  const motif = rng.pick(MOTIFS)

  const svg = `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 ${WIDTH} ${HEIGHT}">`
    + `<defs><linearGradient id="g" x1="0" y1="0" x2="0" y2="1">`
    + `<stop offset="0" stop-color="${tone(palette.hue, 24)}"/>`
    + `<stop offset="1" stop-color="${tone(palette.accentHue, 13)}"/>`
    + `</linearGradient></defs>`
    + `<rect width="${WIDTH}" height="${HEIGHT}" fill="url(#g)"/>`
    + motif(rng, palette)
    + `</svg>`

  return `data:image/svg+xml,${encodeURIComponent(svg)}`
}
