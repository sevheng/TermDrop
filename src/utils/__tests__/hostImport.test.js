import { describe, it, expect } from 'vitest'
import {
  parseHostsFile,
  normalizeImportHost,
  summarizeImport,
  stripMongoPassword,
  parseJumpSpec,
  linkProxyJumps,
  hostRowForUpdate,
} from '../hostImport.js'

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

describe('stripMongoPassword', () => {
  it('removes the password but keeps the username and host', () => {
    expect(stripMongoPassword('mongodb://user:hunter2@localhost:27017/db')).toBe(
      'mongodb://user@localhost:27017/db',
    )
  })

  it('handles srv, seedlists and IPv6', () => {
    expect(stripMongoPassword('mongodb+srv://u:p@cluster.example.net/db')).toBe(
      'mongodb+srv://u@cluster.example.net/db',
    )
    expect(stripMongoPassword('mongodb://u:p@h1:27017,h2:27017/db')).toBe(
      'mongodb://u@h1:27017,h2:27017/db',
    )
    expect(stripMongoPassword('mongodb://u:p@[::1]:27017/db')).toBe(
      'mongodb://u@[::1]:27017/db',
    )
  })

  it('leaves URIs without a password alone', () => {
    for (const uri of [
      'mongodb://localhost:27017/db',
      'mongodb://user@localhost:27017/db',
      'not-a-uri',
    ]) {
      expect(stripMongoPassword(uri)).toBe(uri)
    }
  })

  it('is applied when importing a host', () => {
    const host = normalizeImportHost({
      name: 'db',
      mongo_uri: 'mongodb://admin:secret@localhost:27017',
    })
    expect(host.mongo_uri).not.toContain('secret')
    expect(host.mongo_uri).toBe('mongodb://admin@localhost:27017')
  })
})

describe('parseJumpSpec', () => {
  it('reads every form ssh_config allows for one hop', () => {
    expect(parseJumpSpec('bastion')).toEqual({ user: null, host: 'bastion', port: null })
    expect(parseJumpSpec('ops@bastion')).toEqual({ user: 'ops', host: 'bastion', port: null })
    expect(parseJumpSpec('ops@10.0.0.1:2222')).toEqual({ user: 'ops', host: '10.0.0.1', port: 2222 })
    expect(parseJumpSpec('[fe80::1]:22')).toEqual({ user: null, host: 'fe80::1', port: 22 })
  })
})

describe('linkProxyJumps', () => {
  const row = (id, over = {}) => ({
    id,
    name: `h${id}`,
    host: `10.0.0.${id}`,
    port: 22,
    username: 'u',
    ...over,
  })

  it('links by alias first', () => {
    const hosts = [row(1, { name: 'bastion' }), row(2, { name: 'db' })]
    const imported = [{ name: 'db', host: '10.0.0.2', port: 22, proxy_jump: 'bastion' }]
    const { links, unresolved } = linkProxyJumps(imported, hosts)
    expect(links.map(l => [l.host.id, l.jumpId])).toEqual([[2, 1]])
    expect(unresolved).toEqual([])
  })

  it('falls back to the address, honouring port and user', () => {
    const hosts = [
      row(1, { host: 'jump.example.com', port: 22 }),
      row(3, { host: 'jump.example.com', port: 2222, username: 'ops' }),
      row(2, { name: 'db' }),
    ]
    const imported = [
      { name: 'db', host: '10.0.0.2', port: 22, proxy_jump: 'ops@jump.example.com:2222' },
    ]
    expect(linkProxyJumps(imported, hosts).links[0].jumpId).toBe(3)
  })

  it('reports a hop it cannot match instead of guessing', () => {
    const hosts = [row(2, { name: 'db' })]
    const imported = [{ name: 'db', host: '10.0.0.2', port: 22, proxy_jump: 'nowhere' }]
    const { links, unresolved } = linkProxyJumps(imported, hosts)
    expect(links).toEqual([])
    expect(unresolved[0]).toMatchObject({ name: 'db' })
  })

  it('refuses a chain, including one formed within the same import', () => {
    const hosts = [row(1, { name: 'outer' }), row(2, { name: 'inner' }), row(3, { name: 'db' })]
    const imported = [
      { name: 'inner', host: '10.0.0.2', port: 22, proxy_jump: 'outer' },
      { name: 'db', host: '10.0.0.3', port: 22, proxy_jump: 'inner' },
    ]
    const { links, unresolved } = linkProxyJumps(imported, hosts)
    expect(links.map(l => l.host.name)).toEqual(['inner'])
    expect(unresolved.map(u => u.name)).toEqual(['db'])
  })

  it('never links a host to itself', () => {
    const hosts = [row(1, { name: 'db' })]
    const imported = [{ name: 'db', host: '10.0.0.1', port: 22, proxy_jump: 'db' }]
    expect(linkProxyJumps(imported, hosts).links).toEqual([])
  })
})

describe('hostRowForUpdate', () => {
  it('carries every column, so a one-field change resets nothing', () => {
    const row = {
      id: 5, name: 'db', host: 'h', port: 22, username: 'u', auth_type: 'key', key_path: '/k',
      group: 'prod', favorite: 1, mongo_uri: null, mongo_local_uri: null, redis_uri: null,
      redis_tunnel_host_id: null, jump_host_id: null, created_at: 'x',
    }
    const out = hostRowForUpdate(row, { jump_host_id: 9 })
    expect(out).toMatchObject({ group: 'prod', favorite: 1, key_path: '/k', jump_host_id: 9 })
    expect(out.id).toBeUndefined()
    expect(out.created_at).toBeUndefined()
  })
})
