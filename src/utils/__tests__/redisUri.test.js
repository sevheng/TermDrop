import { describe, it, expect } from 'vitest'
import {
  isRedisUri,
  isTlsRedisUri,
  splitRedisUri,
  stripRedisPassword,
  redisDisplayUri,
  parseRedisUri,
  buildRedisUri,
} from '../redisUri.js'

describe('splitRedisUri', () => {
  it('splits the password-only form Redis actually uses', () => {
    // No username at all: the form most likely to be mishandled.
    expect(splitRedisUri('redis://:hunter2@localhost:6379/0')).toEqual({
      uri: 'redis://@localhost:6379/0',
      password: 'hunter2',
    })
  })

  it('splits an ACL user and password', () => {
    expect(splitRedisUri('redis://default:swordfish@10.0.1.5:6379/2')).toEqual({
      uri: 'redis://default@10.0.1.5:6379/2',
      password: 'swordfish',
    })
  })

  it('leaves the password percent-encoded exactly as it appeared', () => {
    // Re-inserting is then a pure splice with no encoding decisions.
    const { password } = splitRedisUri('rediss://u:p%40ss%3Aw@cache.example.net:6380')
    expect(password).toBe('p%40ss%3Aw')
  })

  it('leaves URIs without a password alone', () => {
    for (const uri of [
      'redis://localhost:6379',
      'redis://default@localhost:6379',
      'redis://:@localhost:6379',
      'not-a-uri',
    ]) {
      expect(splitRedisUri(uri)).toEqual({ uri, password: null })
    }
  })

  it('does not mistake an @ in the path for the separator', () => {
    const uri = 'redis://localhost:6379/0?client=a@b'
    expect(splitRedisUri(uri).password).toBeNull()
  })

  it('handles null', () => {
    expect(splitRedisUri(null)).toEqual({ uri: '', password: null })
  })
})

describe('redisDisplayUri', () => {
  it('masks credentials without hiding the endpoint', () => {
    expect(redisDisplayUri('redis://:pw@10.0.1.5:6379/0')).toBe('redis://***:***@10.0.1.5:6379/0')
    expect(redisDisplayUri('redis://user@h:6379')).toBe('redis://***@h:6379')
    expect(redisDisplayUri('redis://h:6379/0')).toBe('redis://h:6379/0')
  })
})

describe('stripRedisPassword', () => {
  it('is what gets written to the database', () => {
    expect(stripRedisPassword('redis://:pw@h:6379')).toBe('redis://@h:6379')
  })
})

describe('parseRedisUri / buildRedisUri', () => {
  it('round-trips the fields a user typed', () => {
    const cases = [
      'redis://localhost',
      'redis://10.0.1.5:6380',
      'redis://:pw@10.0.1.5:6380/2',
      'rediss://default:pw@cache.example.net:6380/1',
    ]
    for (const uri of cases) {
      expect(buildRedisUri(parseRedisUri(uri))).toBe(uri)
    }
  })

  it('pulls apart a full URI', () => {
    expect(parseRedisUri('rediss://default:pw@cache.example.net:6380/3')).toEqual({
      scheme: 'rediss',
      host: 'cache.example.net',
      port: '6380',
      username: 'default',
      password: 'pw',
      database: '3',
    })
  })

  it('keeps an IPv6 literal intact', () => {
    // The colons inside the brackets are the address, not a port.
    expect(parseRedisUri('redis://[fd00::1]:6380/1')).toMatchObject({
      host: '[fd00::1]',
      port: '6380',
      database: '1',
    })
    expect(parseRedisUri('redis://[fd00::1]')).toMatchObject({ host: '[fd00::1]', port: '' })
  })

  it('omits defaults rather than spelling them out', () => {
    // "redis://h" and "redis://h:6379/0" mean the same thing; the short form
    // is what the user wrote, so do not rewrite it under their cursor.
    expect(buildRedisUri({ host: 'h', port: '6379', database: '0' })).toBe('redis://h')
    expect(buildRedisUri({ host: 'h', port: '6380', database: '2' })).toBe('redis://h:6380/2')
  })

  it('builds the password-only form when there is no username', () => {
    expect(buildRedisUri({ host: 'h', password: 'pw' })).toBe('redis://:pw@h')
  })

  it('returns an empty string with no host', () => {
    expect(buildRedisUri({ host: '' })).toBe('')
    expect(buildRedisUri()).toBe('')
  })

  it('gives empty fields for something that is not a URI', () => {
    expect(parseRedisUri('nonsense')).toMatchObject({ host: '', port: '' })
  })
})

describe('scheme predicates', () => {
  it('recognises redis and rediss', () => {
    expect(isRedisUri('redis://h')).toBe(true)
    expect(isRedisUri('rediss://h')).toBe(true)
    expect(isRedisUri('mongodb://h')).toBe(false)
    expect(isTlsRedisUri('rediss://h')).toBe(true)
    expect(isTlsRedisUri('redis://h')).toBe(false)
  })
})
