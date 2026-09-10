import { describe, it, expect } from 'vitest'
import { splitMongoUri, stripMongoPassword } from '../mongoUri.js'

describe('splitMongoUri', () => {
  it('separates the password and can be rejoined by the backend', () => {
    const { uri, password } = splitMongoUri('mongodb://user:hunter2@localhost:27017/db')
    expect(uri).toBe('mongodb://user@localhost:27017/db')
    expect(password).toBe('hunter2')
  })

  it('keeps the password percent-encoded, matching the Rust side', () => {
    const { password } = splitMongoUri('mongodb://u:p%40ss%3Aw@h:27017/?authSource=admin')
    expect(password).toBe('p%40ss%3Aw')
  })

  it('takes the last @ and the first : so an encoded password survives', () => {
    const { uri, password } = splitMongoUri('mongodb://u:a%40b@h1:27017,h2:27017/db')
    expect(uri).toBe('mongodb://u@h1:27017,h2:27017/db')
    expect(password).toBe('a%40b')
  })

  it('reports no password when there is none to take', () => {
    for (const uri of [
      'mongodb://localhost:27017/db',
      'mongodb://user@localhost:27017/db',
      'mongodb://user:@localhost:27017/db',
      'not-a-uri',
      '',
    ]) {
      expect(splitMongoUri(uri).password).toBeNull()
      expect(splitMongoUri(uri).uri).toBe(uri)
    }
  })

  it('tolerates null and undefined', () => {
    expect(splitMongoUri(null)).toEqual({ uri: '', password: null })
    expect(splitMongoUri(undefined)).toEqual({ uri: '', password: null })
  })
})

describe('stripMongoPassword', () => {
  it('is the uri half of the split', () => {
    expect(stripMongoPassword('mongodb+srv://u:p@c.example.net/db')).toBe(
      'mongodb+srv://u@c.example.net/db',
    )
  })
})
