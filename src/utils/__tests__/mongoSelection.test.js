import { describe, it, expect } from 'vitest'
import {
  isSelected,
  dbSelectionState,
  countSelected,
  buildEntries,
  toggleCollection,
  withoutDb,
  withAllCollections,
} from '../mongoSelection.js'

const db = { name: 'app', collections: ['users', 'logs'] }

describe('mongoSelection', () => {
  it('reports selection state per database', () => {
    expect(dbSelectionState(new Map(), db)).toBe('none')
    expect(dbSelectionState(new Map([['app', new Set(['users'])]]), db)).toBe('some')
    expect(dbSelectionState(new Map([['app', new Set(['users', 'logs'])]]), db)).toBe('all')
    expect(dbSelectionState(new Map([['app', new Set(['x'])]]), { name: 'app', collections: [] })).toBe('some')
  })

  it('toggles collections and drops empty databases', () => {
    let sel = new Map()
    sel = toggleCollection(sel, 'app', 'users')
    expect(isSelected(sel, 'app', 'users')).toBe(true)
    expect(countSelected(sel)).toBe(1)
    sel = toggleCollection(sel, 'app', 'users')
    expect(sel.has('app')).toBe(false)
  })

  it('selects all collections of a database and removes a database', () => {
    let sel = withAllCollections(new Map(), db)
    expect(buildEntries(sel)).toEqual([{ db: 'app', collections: ['users', 'logs'] }])
    sel = withoutDb(sel, 'app')
    expect(buildEntries(sel)).toEqual([])
  })

  it('never mutates the input map', () => {
    const original = new Map([['app', new Set(['users'])]])
    toggleCollection(original, 'app', 'logs')
    withoutDb(original, 'app')
    withAllCollections(original, db)
    expect(buildEntries(original)).toEqual([{ db: 'app', collections: ['users'] }])
  })
})
