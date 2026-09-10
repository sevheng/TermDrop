import { describe, it, expect } from 'vitest'
import { formatBytes, formatRate, formatSpeed } from '../format.js'

describe('formatBytes', () => {
  it('formats each unit boundary', () => {
    expect(formatBytes(0)).toBe('0 B')
    expect(formatBytes(1023)).toBe('1023 B')
    expect(formatBytes(1024)).toBe('1.0 KB')
    expect(formatBytes(1536)).toBe('1.5 KB')
    expect(formatBytes(1024 * 1024)).toBe('1.0 MB')
    expect(formatBytes(1e9)).toBe('953.7 MB')
    expect(formatBytes(1024 * 1024 * 1024)).toBe('1.0 GB')
  })

  it('passes negative and NaN through unchanged in shape', () => {
    expect(formatBytes(-5)).toBe('-5 B')
    expect(formatBytes(NaN)).toBe('NaN GB')
  })
})

describe('formatRate', () => {
  it('formats by magnitude and keeps the sign', () => {
    expect(formatRate(500)).toBe('500 B/s')
    expect(formatRate(-500)).toBe('-500 B/s')
    expect(formatRate(2048)).toBe('2.0 KB/s')
    expect(formatRate(-2048)).toBe('-2.0 KB/s')
    expect(formatRate(3 * 1024 * 1024)).toBe('3.0 MB/s')
  })
})

describe('formatSpeed', () => {
  it('matches formatRate for non-negative input', () => {
    for (const v of [0, 500, 1024, 2048, 5 * 1024 * 1024]) {
      expect(formatSpeed(v)).toBe(formatRate(v))
    }
  })

  it('does not scale negative input (documented difference from formatRate)', () => {
    expect(formatSpeed(-2048)).toBe('-2048 B/s')
  })
})
