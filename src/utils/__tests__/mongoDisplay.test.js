import { describe, it, expect } from 'vitest'
import { mongoDisplayUri, stripCredentials } from '../mongoDisplay.js'

describe('mongoDisplayUri', () => {
  it('reduces a URI to host and port', () => {
    expect(mongoDisplayUri('mongodb://user:pass@localhost:27017/db')).toBe(
      'mongodb://localhost:27017',
    )
  })

  it('keeps the srv scheme and drops the port', () => {
    expect(mongoDisplayUri('mongodb+srv://u:p@cluster.example.net/db')).toBe(
      'mongodb+srv://cluster.example.net',
    )
  })

  it('never renders credentials, even for an unparseable URI', () => {
    const messy = 'mongodb://admin:hunter2@h1:27017,h2:27017/db'
    const shown = mongoDisplayUri(messy)
    expect(shown).not.toContain('hunter2')
  })

  it('returns an empty string for nothing', () => {
    expect(mongoDisplayUri('')).toBe('')
    expect(mongoDisplayUri(null)).toBe('')
    expect(mongoDisplayUri(undefined)).toBe('')
  })
})

describe('stripCredentials', () => {
  it('removes userinfo but keeps the scheme and host', () => {
    expect(stripCredentials('mongodb://u:p@h1:27017,h2:27017/db')).toBe(
      'mongodb://h1:27017,h2:27017/db',
    )
  })

  it('truncates something very long', () => {
    const long = 'mongodb://' + 'h'.repeat(80)
    expect(stripCredentials(long).endsWith('…')).toBe(true)
  })
})
