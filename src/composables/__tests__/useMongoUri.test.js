import { describe, it, expect } from 'vitest'
import { parseMongoUri, buildMongoUri, parseUriToForm } from '../useMongoUri.js'

describe('parseMongoUri', () => {
  it('returns form defaults for empty input', () => {
    expect(parseMongoUri('')).toEqual({
      mode: 'form',
      scheme: 'mongodb',
      host: '',
      port: 27017,
      username: '',
      password: '',
      database: '',
      authSource: 'admin',
      options: '',
    })
    expect(parseMongoUri(null)).toEqual(parseMongoUri(''))
  })

  it('parses a full single-host URI into form fields', () => {
    expect(
      parseMongoUri('mongodb://user:p%40ss@db.example.com:27018/mydb?authSource=admin&retryWrites=true'),
    ).toEqual({
      mode: 'form',
      scheme: 'mongodb',
      host: 'db.example.com',
      port: 27018,
      username: 'user',
      password: 'p@ss',
      database: 'mydb',
      authSource: 'admin',
      options: 'retryWrites=true',
    })
  })

  it('applies defaults for port, database, and authSource', () => {
    const r = parseMongoUri('mongodb://localhost')
    expect(r.port).toBe(27017)
    expect(r.database).toBe('')
    expect(r.authSource).toBe('admin')
    expect(r.options).toBe('')
  })

  it('keeps a non-default authSource and strips it from options', () => {
    const r = parseMongoUri('mongodb://u:p@h/d?authSource=other&w=majority')
    expect(r.authSource).toBe('other')
    expect(r.options).toBe('w=majority')
  })

  it('keeps SRV URIs raw', () => {
    const uri = 'mongodb+srv://u:p@cluster.example.com/db'
    expect(parseMongoUri(uri)).toEqual({ mode: 'uri', scheme: 'mongodb+srv', uri })
  })

  it('keeps multi-host replica-set URIs raw', () => {
    const uri = 'mongodb://a:1,b:2/db?replicaSet=rs0'
    expect(parseMongoUri(uri)).toEqual({ mode: 'uri', scheme: 'mongodb', uri })
  })
})

describe('buildMongoUri', () => {
  it('omits default port, empty database, and default authSource', () => {
    expect(buildMongoUri({ host: 'localhost', port: 27017 })).toBe('mongodb://localhost')
  })

  it('includes credentials, port, database, and merged options', () => {
    expect(
      buildMongoUri({
        host: 'h',
        port: 27018,
        username: 'u',
        password: 'p@ss',
        database: 'd',
        authSource: 'auth',
        options: 'retryWrites=true&authSource=ignored',
      }),
    ).toBe('mongodb://u:p%40ss@h:27018/d?authSource=auth&retryWrites=true')
  })

  it('brackets IPv6 hosts whether or not they arrive bracketed', () => {
    expect(buildMongoUri({ host: '::1', port: 27017 })).toBe('mongodb://[::1]')
    expect(buildMongoUri({ host: '[::1]', port: 27017 })).toBe('mongodb://[::1]')
  })

  it('supports the SRV scheme and a username without password', () => {
    expect(
      buildMongoUri({ scheme: 'mongodb+srv', host: 'c.example.com', port: 27017, username: 'u', database: 'd' }),
    ).toBe('mongodb+srv://u@c.example.com/d')
  })

  it('round-trips through parseMongoUri', () => {
    const fields = {
      host: 'db.example.com',
      port: 27018,
      username: 'user',
      password: 'p@ss:word',
      database: 'mydb',
      authSource: 'custom',
      options: 'retryWrites=true',
    }
    const parsed = parseMongoUri(buildMongoUri(fields))
    expect(parsed).toMatchObject({ mode: 'form', ...fields })
  })
})

describe('parseUriToForm', () => {
  it('rejects non-mongo and multi-host URIs', () => {
    expect(parseUriToForm('')).toBeNull()
    expect(parseUriToForm('http://x')).toBeNull()
    expect(parseUriToForm('mongodb://a:1,b:2/db')).toBeNull()
  })

  it('parses a single-host URI', () => {
    expect(parseUriToForm('mongodb://u:p@h:27018/d?authSource=x&w=majority')).toEqual({
      host: 'h',
      port: 27018,
      username: 'u',
      password: 'p',
      database: 'd',
      authSource: 'x',
      options: 'w=majority',
    })
  })

  it('parses an SRV URI with defaults', () => {
    expect(parseUriToForm('mongodb+srv://u@c.example.com/d')).toEqual({
      host: 'c.example.com',
      port: 27017,
      username: 'u',
      password: '',
      database: 'd',
      authSource: 'admin',
      options: '',
    })
  })
})
