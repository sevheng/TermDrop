import { describe, it, expect } from 'vitest'
import { GROUP_ACCENTS, groupAccentIndex, groupAccentClass } from '../groupAccent.js'

describe('groupAccentIndex', () => {
  it('always lands in range', () => {
    for (const n of ['', 'a', 'Production', 'staging-eu-west-1', '日本語', '1', 'x'.repeat(500)]) {
      const i = groupAccentIndex(n)
      expect(i).toBeGreaterThanOrEqual(0)
      expect(i).toBeLessThan(GROUP_ACCENTS.length)
    }
  })

  it('is stable for the same name', () => {
    expect(groupAccentIndex('Production')).toBe(groupAccentIndex('Production'))
  })

  it('is a property of the name, not the position', () => {
    // Deleting or renaming the group above must not recolour this one, which
    // is the whole reason this hashes rather than counting.
    const before = groupAccentIndex('databases')
    const others = ['web', 'cache', 'staging'].map(groupAccentIndex)
    expect(groupAccentIndex('databases')).toBe(before)
    expect(others).toHaveLength(3)
  })

  it('does not throw on null or undefined', () => {
    expect(() => groupAccentIndex(null)).not.toThrow()
    expect(() => groupAccentIndex(undefined)).not.toThrow()
  })

  it('spreads a realistic set of names over several hues', () => {
    const names = ['web', 'db', 'cache', 'staging', 'prod', 'eu', 'us', 'dev']
    const used = new Set(names.map(groupAccentIndex))
    expect(used.size).toBeGreaterThan(1)
  })
})

describe('groupAccentClass', () => {
  it('returns a real Tailwind class, spelled out in full', () => {
    const c = groupAccentClass('Production')
    expect(GROUP_ACCENTS).toContain(c)
    // Interpolated class names compile to nothing, so this must be literal.
    expect(c).toMatch(/^border-tag-[1-8]$/)
  })
})
