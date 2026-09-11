import { describe, it, expect } from 'vitest'
import { readFileSync } from 'node:fs'

/**
 * The launch splash in index.html is the one place outside themes/tokens.css
 * that names a colour, because it has to paint before that stylesheet has
 * loaded. Nothing can make it read the tokens at that moment, so the values
 * are duplicated -- and this is what stops them drifting on the next palette
 * change, which would reintroduce the launch flash they exist to prevent.
 *
 * The Rust half of the same duplication is covered by
 * `canvas_colours_match_the_design_tokens` in src-tauri/src/main.rs.
 */
const html = readFileSync('index.html', 'utf8')
const tokens = readFileSync('src/themes/tokens.css', 'utf8')
const icon = readFileSync('src-tauri/icons/src/icon-small.svg', 'utf8')

/** `--td-name: R G B;` for each theme block, in source order: light, then dark. */
function channelsOf(name) {
  return [...tokens.matchAll(new RegExp(`--${name}:\\s*([\\d\\s]+);`, 'g'))].map(m => {
    const [r, g, b] = m[1].trim().split(/\s+/).map(Number)
    return '#' + [r, g, b].map(n => n.toString(16).padStart(2, '0')).join('')
  })
}

describe('launch splash', () => {
  it('finds the splash and the tokens to check', () => {
    // Guards the guard: a regex that matched nothing would let every
    // assertion below pass by checking nothing at all.
    expect(html).toContain('id="td-splash"')
    expect(channelsOf('td-canvas')).toHaveLength(2)
    expect(channelsOf('td-ink')).toHaveLength(2)
  })

  it('paints the same canvas and ink as the design tokens', () => {
    for (const hex of [...channelsOf('td-canvas'), ...channelsOf('td-ink')]) {
      expect(html.toLowerCase()).toContain(hex.toLowerCase())
    }
  })

  it('draws the droplet in the app icon’s own colours', () => {
    // The mark is lifted from the icon master, so the droplet gradient has to
    // match it -- the splash is the app's first impression of its own icon.
    const drop = icon.match(/<linearGradient id="drop"[\s\S]*?<\/linearGradient>/)
    expect(drop).not.toBeNull()
    const stops = [...drop[0].matchAll(/stop-color="(#[0-9A-Fa-f]{6})"/g)].map(m => m[1])
    expect(stops).toHaveLength(2)
    for (const stop of stops) expect(html).toContain(stop)
  })

  it('keeps the app itself free of the opacity that would break fixed positioning', () => {
    // Fading #app would make it a containing block for the app's many
    // position:fixed menus and modals. Only the splash may animate.
    expect(html).not.toMatch(/#app\s*\{[^}]*opacity/)
  })
})
