import { describe, it, expect } from 'vitest'
import {
  validateJsonInput,
  clampPageSize,
  skipFor,
  lastPage,
  describeRange,
  MAX_PAGE_SIZE,
  DEFAULT_PAGE_SIZE,
} from '../mongoQuery.js'

describe('validateJsonInput', () => {
  it('accepts an empty box and a valid object', () => {
    expect(validateJsonInput('')).toBeNull()
    expect(validateJsonInput('   ')).toBeNull()
    expect(validateJsonInput('{"a":1}')).toBeNull()
  })

  it('rejects malformed JSON, naming the field', () => {
    expect(validateJsonInput('{a:1}', 'sort')).toMatch(/sort is not valid JSON/)
  })

  it('rejects valid JSON that is not an object', () => {
    expect(validateJsonInput('[1,2]')).toMatch(/must be a JSON object/)
    expect(validateJsonInput('42')).toMatch(/must be a JSON object/)
    expect(validateJsonInput('null')).toMatch(/must be a JSON object/)
  })
})

describe('clampPageSize', () => {
  it('never exceeds what the backend returns', () => {
    expect(clampPageSize(10_000)).toBe(MAX_PAGE_SIZE)
  })

  it('falls back to the default for nonsense', () => {
    expect(clampPageSize(0)).toBe(DEFAULT_PAGE_SIZE)
    expect(clampPageSize(-5)).toBe(DEFAULT_PAGE_SIZE)
    expect(clampPageSize('abc')).toBe(DEFAULT_PAGE_SIZE)
  })
})

describe('skipFor', () => {
  it('is page times page size', () => {
    expect(skipFor(0, 25)).toBe(0)
    expect(skipFor(3, 25)).toBe(75)
  })

  it('treats a negative page as the first', () => {
    expect(skipFor(-2, 25)).toBe(0)
  })
})

describe('lastPage', () => {
  it('is zero-based and accounts for a partial final page', () => {
    expect(lastPage(100, 25)).toBe(3)
    expect(lastPage(101, 25)).toBe(4)
  })

  it('is page 0 when there is nothing', () => {
    expect(lastPage(0, 25)).toBe(0)
  })
})

describe('describeRange', () => {
  it('describes the current window', () => {
    expect(describeRange(0, 25, 25, 100, false)).toBe('1–25 of 100')
    expect(describeRange(2, 25, 10, 60, false)).toBe('51–60 of 60')
  })

  it('marks an estimated total', () => {
    expect(describeRange(0, 25, 25, 10000000, true)).toContain('~10,000,000')
  })

  it('has an empty state', () => {
    expect(describeRange(0, 25, 0, 0, false)).toBe('No documents')
    expect(describeRange(9, 25, 0, 100, false)).toBe('No documents on this page')
  })
})
