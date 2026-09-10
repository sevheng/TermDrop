import { describe, it, expect } from 'vitest'
import { parseHostsFile, normalizeImportHost, summarizeImport } from '../hostImport.js'

describe('parseHostsFile', () => {
  it('accepts a bare array, which is what export writes', () => {
    expect(parseHostsFile('[{"name":"a"}]')).toEqual([{ name: 'a' }])
    expect(parseHostsFile('[]')).toEqual([])
  })

  it('accepts a hosts envelope', () => {
    expect(parseHostsFile('{"version":1,"hosts":[{"name":"a"}]}')).toEqual([{ name: 'a' }])
  })

  it('explains what is wrong instead of leaking a parser error', () => {
    expect(() => parseHostsFile('not json')).toThrow('not valid JSON')
    expect(() => parseHostsFile('{"a":1}')).toThrow('host export')
    expect(() => parseHostsFile('[1,2]')).toThrow('not a host')
    expect(() => parseHostsFile('[null]')).toThrow('not a host')
  })
})

describe('normalizeImportHost', () => {
  it('keeps only known fields, so a stray password never reaches the backend', () => {
    const out = normalizeImportHost({ name: 'a', host: 'h', password: 'secret', id: 7 })
    expect(out).not.toHaveProperty('password')
    expect(out).not.toHaveProperty('id')
    expect(Object.keys(out).sort()).toEqual([
      'auth_type', 'favorite', 'group', 'host', 'key_path',
      'mongo_local_uri', 'mongo_uri', 'name', 'port', 'username',
    ])
  })

  it('coerces and defaults the scalar fields', () => {
    const out = normalizeImportHost({ name: '  a  ', host: ' h ', port: '2222', favorite: true })
    expect(out.name).toBe('a')
    expect(out.host).toBe('h')
    expect(out.port).toBe(2222)
    expect(out.favorite).toBe(1)
    expect(out.auth_type).toBe('password')
    expect(normalizeImportHost({}).port).toBe(22)
    expect(normalizeImportHost({ port: 'abc' }).port).toBe(22)
    expect(normalizeImportHost({ auth_type: 'key' }).auth_type).toBe('key')
    expect(normalizeImportHost({ auth_type: 'agent' }).auth_type).toBe('password')
  })

  it('normalizes absent optional fields to null', () => {
    const out = normalizeImportHost({ name: 'a' })
    expect(out.key_path).toBeNull()
    expect(out.group).toBeNull()
    expect(out.mongo_uri).toBeNull()
  })
})

describe('summarizeImport', () => {
  it('pluralizes and omits zero counts', () => {
    expect(summarizeImport({ added: 1 })).toEqual({ message: 'Imported 1 host', type: 'success' })
    expect(summarizeImport({ added: 3 }).message).toBe('Imported 3 hosts')
    expect(summarizeImport({ added: 2, replaced: 1 }).message).toBe('Imported 2 hosts, replaced 1 host')
    expect(summarizeImport({ added: 1, skipped: 4 }).message).toBe('Imported 1 host, skipped 4')
  })

  it('names the first failure and counts the rest', () => {
    const one = summarizeImport({ added: 1, failed: [{ name: 'bad', reason: 'Port' }] })
    expect(one.type).toBe('warning')
    expect(one.message).toBe('Imported 1 host, 1 failure: bad (Port)')

    const many = summarizeImport({
      failed: [{ name: 'a', reason: 'Port' }, { name: 'b', reason: 'Name' }],
    })
    expect(many.message).toBe('2 failures: a (Port) and 1 more')
  })

  it('says so when nothing happened', () => {
    expect(summarizeImport({})).toEqual({ message: 'Nothing to import', type: 'info' })
  })
})
