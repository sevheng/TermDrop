import { describe, it, expect } from 'vitest'
import { resolveSides, sideLabel } from '../mongoDirection.js'

describe('resolveSides', () => {
  it('uses the remote connection for every role when it is the only one', () => {
    // The regression this module exists for: restore used to resolve to the
    // empty local URI here, which disabled it for single-connection hosts.
    expect(resolveSides(false, true)).toEqual({ source: 'remote', dest: 'remote' })
    // The direction toggle is hidden without a local connection, but the
    // answer must not depend on the flag either way.
    expect(resolveSides(false, false)).toEqual({ source: 'remote', dest: 'remote' })
  })

  it('follows the direction when both connections exist', () => {
    expect(resolveSides(true, true)).toEqual({ source: 'remote', dest: 'local' })
    expect(resolveSides(true, false)).toEqual({ source: 'local', dest: 'remote' })
  })

  it('never reports the same side for both roles when two connections exist', () => {
    for (const isRemoteToLocal of [true, false]) {
      const { source, dest } = resolveSides(true, isRemoteToLocal)
      expect(source).not.toBe(dest)
    }
  })

  it('defaults to restoring into remote, as it did before the direction toggle', () => {
    // Anyone who never flips direction must see no change in behaviour.
    expect(resolveSides(true, true).source).toBe('remote')
  })
})

describe('sideLabel', () => {
  it('names each side for a button', () => {
    expect(sideLabel('remote')).toBe('Remote')
    expect(sideLabel('local')).toBe('Local')
  })
})
