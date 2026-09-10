import { describe, it, expect } from 'vitest'
import { readFileSync, readdirSync, statSync } from 'node:fs'
import { join } from 'node:path'
import { findBannedClasses, formatFindings } from '../utils/classGuard.js'

/**
 * Asserts the whole component tree against the design-system rules.
 *
 * Runs under the existing `npm test`, so CI enforces it with no workflow
 * change. When this fails it names the file, the line and the replacement.
 */
function walk(dir, out = []) {
  for (const entry of readdirSync(dir)) {
    const full = join(dir, entry)
    if (statSync(full).isDirectory()) walk(full, out)
    else if (full.endsWith('.vue')) out.push(full)
  }
  return out
}

const files = walk('src')

describe('design system guard', () => {
  it('finds the components to check', () => {
    // Guards the guard: a broken glob would make the assertion below pass
    // by checking nothing at all.
    expect(files.length).toBeGreaterThan(20)
  })

  it('no component leaks a palette colour, a white label on a bright fill, or a bracket type size', () => {
    const failures = []
    for (const file of files) {
      const findings = findBannedClasses(readFileSync(file, 'utf8'))
      if (findings.length) failures.push(formatFindings(file, findings))
    }
    expect(failures.join('\n')).toBe('')
  })
})

describe('findBannedClasses', () => {
  it('catches a palette class and reports where', () => {
    const hits = findBannedClasses('<div class="text-gray-400">\n<p class="bg-red-500">')
    expect(hits).toHaveLength(2)
    expect(hits[0]).toMatchObject({ match: 'text-gray-400', line: 1 })
    expect(hits[1]).toMatchObject({ match: 'bg-red-500', line: 2 })
  })

  it('catches a bright fill carrying a white label', () => {
    const hits = findBannedClasses('<button class="bg-accent text-white">')
    expect(hits.map(h => h.rule)).toContain('bright-fill-white-label')
  })

  it('allows the -solid variants, which exist to carry a label', () => {
    expect(findBannedClasses('<button class="bg-accent-solid text-white">')).toEqual([])
    expect(findBannedClasses('<button class="bg-good-solid text-white">')).toEqual([])
  })

  it('allows role tokens and a bright tone used as text', () => {
    expect(findBannedClasses('<p class="text-ink-2 bg-surface border-line">')).toEqual([])
    expect(findBannedClasses('<p class="text-accent">')).toEqual([])
    expect(findBannedClasses('<span class="bg-good w-2 h-2 rounded-full">')).toEqual([])
  })

  it('catches a bracket font size, which carries no line-height', () => {
    const hits = findBannedClasses('<p class="text-[10px]">')
    expect(hits.map(h => h.rule)).toContain('arbitrary-type-size')
  })

  it('allows the named scale tiers', () => {
    expect(findBannedClasses('<p class="text-2xs">')).toEqual([])
    expect(findBannedClasses('<p class="text-xs">')).toEqual([])
  })

  it('does not confuse a token that merely contains a hue name', () => {
    // `border-tag-3` and `text-redis` must not trip the hue matcher.
    expect(findBannedClasses('<span class="border-tag-3 text-redis">')).toEqual([])
  })
})
