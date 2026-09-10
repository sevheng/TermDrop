import { describe, it, expect } from 'vitest'
import {
  HOST_KIND,
  hostKind,
  isMongoOnlyHost,
  isRedisOnlyHost,
  isDatastoreHost,
  activateVerb,
} from '../hostKind.js'

const ssh = { name: 's', host: '10.0.0.1', port: 22 }
const mongo = { name: 'm', host: '', mongo_uri: 'mongodb://h/db' }
const redis = { name: 'r', host: '', redis_uri: 'redis://h:6379/0' }

describe('hostKind', () => {
  it('names each kind', () => {
    expect(hostKind(ssh)).toBe(HOST_KIND.SSH)
    expect(hostKind(mongo)).toBe(HOST_KIND.MONGODB)
    expect(hostKind(redis)).toBe(HOST_KIND.REDIS)
  })

  it('treats a row with an SSH host as SSH even when it carries a URI', () => {
    // The URI belongs to the machine, not instead of it.
    expect(hostKind({ host: '10.0.0.1', redis_uri: 'redis://h:6379' })).toBe(HOST_KIND.SSH)
    expect(hostKind({ host: '10.0.0.1', mongo_uri: 'mongodb://h' })).toBe(HOST_KIND.SSH)
  })

  it('resolves a row carrying both URIs deterministically', () => {
    const both = { host: '', mongo_uri: 'mongodb://h', redis_uri: 'redis://h:6379' }
    expect(hostKind(both)).toBe(HOST_KIND.REDIS)
  })

  it('does not throw on a missing host', () => {
    expect(hostKind(null)).toBe(HOST_KIND.SSH)
    expect(hostKind(undefined)).toBe(HOST_KIND.SSH)
  })
})

describe('predicates', () => {
  it('are mutually exclusive', () => {
    expect(isMongoOnlyHost(mongo)).toBe(true)
    expect(isRedisOnlyHost(mongo)).toBe(false)
    expect(isRedisOnlyHost(redis)).toBe(true)
    expect(isMongoOnlyHost(redis)).toBe(false)
    expect(isDatastoreHost(ssh)).toBe(false)
    expect(isDatastoreHost(redis)).toBe(true)
  })
})

describe('activateVerb', () => {
  it('says Connect for a machine and Open for a datastore', () => {
    expect(activateVerb(ssh)).toBe('Connect')
    expect(activateVerb(redis)).toBe('Open')
    expect(activateVerb(mongo)).toBe('Open')
  })
})
