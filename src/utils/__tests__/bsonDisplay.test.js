import { describe, it, expect } from 'vitest'
import {
  unwrapExtendedJson,
  toDisplayValue,
  prettyPrintDocument,
  summarizeDocument,
} from '../bsonDisplay.js'

describe('unwrapExtendedJson', () => {
  it('renders an ObjectId the way the shell does', () => {
    expect(unwrapExtendedJson({ $oid: '507f1f77bcf86cd799439011' })).toBe(
      "ObjectId('507f1f77bcf86cd799439011')",
    )
  })

  it('renders a date from the canonical nested form', () => {
    expect(unwrapExtendedJson({ $date: { $numberLong: '1577836800000' } })).toBe(
      "ISODate('2020-01-01T00:00:00.000Z')",
    )
  })

  it('keeps big and decimal numbers as text so no precision is lost', () => {
    // 2^53 + 1 cannot be represented exactly as a JS number.
    expect(unwrapExtendedJson({ $numberLong: '9007199254740993' })).toBe('9007199254740993')
    expect(unwrapExtendedJson({ $numberDecimal: '1.10' })).toBe('1.10')
  })

  it('returns null for values that are not wrappers', () => {
    expect(unwrapExtendedJson({ a: 1 })).toBeNull()
    expect(unwrapExtendedJson([1, 2])).toBeNull()
    expect(unwrapExtendedJson('plain')).toBeNull()
    expect(unwrapExtendedJson(null)).toBeNull()
  })
})

describe('toDisplayValue', () => {
  it('unwraps nested structures', () => {
    const doc = {
      _id: { $oid: '507f1f77bcf86cd799439011' },
      meta: { at: { $date: { $numberLong: '0' } }, tags: [{ $numberInt: '1' }] },
    }
    expect(toDisplayValue(doc)).toEqual({
      _id: "ObjectId('507f1f77bcf86cd799439011')",
      meta: { at: "ISODate('1970-01-01T00:00:00.000Z')", tags: ['1'] },
    })
  })

  it('stops at the depth limit rather than recursing forever', () => {
    let deep = { v: 1 }
    for (let i = 0; i < 30; i++) deep = { nested: deep }
    expect(JSON.stringify(toDisplayValue(deep, 0, 5))).toContain('…')
  })
})

describe('prettyPrintDocument', () => {
  it('produces indented, unwrapped JSON', () => {
    const out = prettyPrintDocument('{"_id":{"$oid":"507f1f77bcf86cd799439011"}}')
    expect(out).toContain("ObjectId('507f1f77bcf86cd799439011')")
    // and no escaped-quote noise from JSON.stringify
    expect(out).not.toContain('\\"')
    expect(out).toContain('\n')
  })

  it('returns the input unchanged when it cannot be parsed', () => {
    expect(prettyPrintDocument('not json')).toBe('not json')
  })
})

describe('summarizeDocument', () => {
  it('leads with _id', () => {
    const line = summarizeDocument('{"name":"n","_id":{"$oid":"507f1f77bcf86cd799439011"}}')
    expect(line.startsWith('_id: ObjectId(')).toBe(true)
  })

  it('collapses nested values and truncates', () => {
    const line = summarizeDocument('{"a":{"b":1},"c":[1,2,3]}')
    expect(line).toContain('a: {…}')
    expect(line).toContain('c: [3]')

    const long = summarizeDocument(JSON.stringify({ x: 'y'.repeat(500) }), 40)
    expect(long.length).toBeLessThanOrEqual(40)
  })
})
