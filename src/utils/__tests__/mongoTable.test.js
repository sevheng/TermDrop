import { describe, it, expect } from 'vitest'
import {
  parseDocuments,
  deriveColumns,
  cellText,
  DEFAULT_MAX_COLUMNS,
} from '../mongoTable.js'

const oid = '507f1f77bcf86cd799439011'

describe('parseDocuments', () => {
  it('unwraps extended JSON the same way the JSON view does', () => {
    const [doc] = parseDocuments([`{"_id":{"$oid":"${oid}"},"n":1}`])
    expect(doc._id).toBe(`ObjectId('${oid}')`)
    expect(doc.n).toBe(1)
  })

  it('keeps big and decimal numbers exact rather than rounding them', () => {
    const [doc] = parseDocuments([
      '{"big":{"$numberLong":"9007199254740993"},"dec":{"$numberDecimal":"1.10"}}',
    ])
    // 2^53 + 1 would lose its last digit as a JS number.
    expect(doc.big).toBe('9007199254740993')
    expect(doc.dec).toBe('1.10')
  })

  it('turns an unparseable document into null instead of losing the page', () => {
    const docs = parseDocuments(['{"a":1}', 'not json', '{"b":2}'])
    expect(docs).toHaveLength(3)
    expect(docs[1]).toBeNull()
    expect(docs[0].a).toBe(1)
    expect(docs[2].b).toBe(2)
  })

  it('treats a non-object document as having no fields', () => {
    expect(parseDocuments(['42', '[1,2]'])).toEqual([null, null])
  })

  it('tolerates no input', () => {
    expect(parseDocuments()).toEqual([])
    expect(parseDocuments([])).toEqual([])
  })
})

describe('deriveColumns', () => {
  it('leads with _id and then the fields most documents share', () => {
    const docs = [
      { _id: 'a', common: 1, rare: 1 },
      { _id: 'b', common: 2 },
      { _id: 'c', common: 3 },
    ]
    expect(deriveColumns(docs).columns).toEqual(['_id', 'common', 'rare'])
  })

  it('unions differing shapes so no field is dropped', () => {
    const docs = [{ a: 1 }, { b: 2 }, { c: 3 }]
    expect(deriveColumns(docs).columns.sort()).toEqual(['a', 'b', 'c'])
  })

  it('breaks ties by first appearance so the order is stable', () => {
    const docs = [{ first: 1, second: 2 }, { second: 2, first: 1 }]
    expect(deriveColumns(docs).columns).toEqual(['first', 'second'])
  })

  it('caps the columns and reports how many were hidden', () => {
    const wide = [Object.fromEntries(Array.from({ length: 20 }, (_, i) => [`f${i}`, i]))]
    const { columns, hidden } = deriveColumns(wide, 5)
    expect(columns).toHaveLength(5)
    expect(hidden).toBe(15)
  })

  it('hides nothing when everything fits', () => {
    expect(deriveColumns([{ a: 1, b: 2 }]).hidden).toBe(0)
    expect(DEFAULT_MAX_COLUMNS).toBeGreaterThan(2)
  })

  it('omits _id when no document has one', () => {
    expect(deriveColumns([{ a: 1 }]).columns).toEqual(['a'])
  })

  it('skips nulls from unparseable documents', () => {
    expect(deriveColumns([null, { a: 1 }, null]).columns).toEqual(['a'])
    expect(deriveColumns([]).columns).toEqual([])
    expect(deriveColumns(undefined).columns).toEqual([])
  })
})

describe('cellText', () => {
  it('renders scalars as text', () => {
    expect(cellText('x')).toBe('x')
    expect(cellText(0)).toBe('0')
    expect(cellText(false)).toBe('false')
  })

  it('distinguishes a missing field from a null one', () => {
    // A document simply lacking the column should leave the cell blank, not
    // claim the value is null.
    expect(cellText(undefined)).toBe('')
    expect(cellText(null)).toBe('null')
  })

  it('collapses nested values, which the expanded row shows in full', () => {
    expect(cellText({ a: 1 })).toBe('{…}')
    expect(cellText([1, 2, 3])).toBe('[3]')
    expect(cellText([])).toBe('[0]')
  })

  it('passes through the unwrapped ObjectId and date forms unchanged', () => {
    expect(cellText(`ObjectId('${oid}')`)).toBe(`ObjectId('${oid}')`)
    expect(cellText("ISODate('2020-01-01T00:00:00.000Z')")).toBe(
      "ISODate('2020-01-01T00:00:00.000Z')",
    )
  })
})
