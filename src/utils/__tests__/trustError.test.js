import { describe, it, expect } from 'vitest'
import {
  isTrustError,
  parseTrustError,
  hopOf,
  shouldAskToTrust,
  TRUST_PREFIX,
} from '../trustError.js'

const payload = (over = {}) =>
  TRUST_PREFIX +
  JSON.stringify({
    kind: 'unknown',
    host: 'example.com',
    port: 22,
    fingerprint: 'SHA256:abc',
    keyType: 'ssh-ed25519',
    message: '',
    ...over,
  })

describe('isTrustError', () => {
  it('matches only at the very start of the error', () => {
    // The spoofing guard. A hostname, a server banner or a remote path could
    // contain the sentinel, and `includes` would let that text conjure a
    // "trust this host" dialog for a key the backend never verified.
    expect(isTrustError(payload())).toBe(true)
    expect(isTrustError(`connect to host ${TRUST_PREFIX}{"kind":"unknown"} failed`)).toBe(false)
    expect(isTrustError(`ssh: ${TRUST_PREFIX}`)).toBe(false)
  })

  it('is false for ordinary errors', () => {
    expect(isTrustError('auth: Authentication failed')).toBe(false)
    expect(isTrustError('')).toBe(false)
    expect(isTrustError(null)).toBe(false)
  })
})

describe('parseTrustError', () => {
  it('returns the fields the dialog needs', () => {
    expect(parseTrustError(payload())).toMatchObject({
      kind: 'unknown',
      host: 'example.com',
      port: 22,
      fingerprint: 'SHA256:abc',
      keyType: 'ssh-ed25519',
    })
  })

  it('carries the explanation for a refusal', () => {
    const parsed = parseTrustError(payload({ kind: 'refused', message: 'it changed' }))
    expect(parsed.kind).toBe('refused')
    expect(parsed.message).toBe('it changed')
  })

  it('refuses anything it cannot fully understand', () => {
    // A malformed payload must not become a dialog inviting the user to trust
    // something we cannot even describe.
    expect(parseTrustError(TRUST_PREFIX + 'not json')).toBe(null)
    expect(parseTrustError(payload({ kind: 'something-else' }))).toBe(null)
    expect(parseTrustError(payload({ fingerprint: '' }))).toBe(null)
    expect(parseTrustError(payload({ fingerprint: undefined }))).toBe(null)
    expect(parseTrustError('auth: failed')).toBe(null)
  })
})

describe('shouldAskToTrust', () => {
  it('asks once for an unknown host', () => {
    expect(shouldAskToTrust(payload(), {})).toMatchObject({ host: 'example.com' })
  })

  it('never asks twice', () => {
    // If a fingerprint was just accepted and the host still reads as unknown,
    // the key changed between attempts. Asking again would walk the user into
    // accepting a different server than the one they were shown.
    expect(shouldAskToTrust(payload(), { target: 'SHA256:abc' })).toBe(null)
  })

  it('never offers to trust a refusal', () => {
    // The whole point: a changed or revoked key has no path through, and the
    // acceptance flow must not be reachable from one.
    expect(shouldAskToTrust(payload({ kind: 'refused' }), {})).toBe(null)
  })
})

describe('hops', () => {
  it('reads the hop, and treats a payload without one as the target', () => {
    expect(hopOf({ hop: 'jump' })).toBe('jump')
    expect(hopOf({ hop: 'target' })).toBe('target')
    expect(hopOf({})).toBe('target')
    expect(hopOf({ hop: 'anything else' })).toBe('target')
  })

  it('asks about the target after the jump host was accepted', () => {
    // A jumped host legitimately needs two answers: bastion, then target.
    const accepted = { jump: 'SHA256:bastion' }
    expect(shouldAskToTrust(payload({ hop: 'target' }), accepted)).toMatchObject({
      host: 'example.com',
    })
  })

  it('never asks about the same hop twice', () => {
    const accepted = { jump: 'SHA256:bastion' }
    expect(shouldAskToTrust(payload({ hop: 'jump' }), accepted)).toBe(null)
  })
})
