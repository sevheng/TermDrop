import { describe, it, expect } from 'vitest'
import {
  escapeGlob,
  prefixPattern,
  createCursorStack,
  formatTtl,
  formatKeyKind,
  isViewableKind,
  summarizeScan,
} from '../redisKeys.js'

describe('escapeGlob', () => {
  it('escapes every Redis glob metacharacter', () => {
    // Unescaped, a prefix like "a*" would match half the keyspace and a
    // prefix like "report[2024]" would match nothing.
    expect(escapeGlob('a*b')).toBe('a\\*b')
    expect(escapeGlob('a?b')).toBe('a\\?b')
    expect(escapeGlob('report[2024]')).toBe('report\\[2024\\]')
    expect(escapeGlob('a\\b')).toBe('a\\\\b')
  })

  it('leaves an ordinary prefix alone', () => {
    expect(escapeGlob('user')).toBe('user')
  })
})

describe('prefixPattern', () => {
  it('turns a group into a server-side MATCH', () => {
    expect(prefixPattern('user')).toBe('user:*')
    expect(prefixPattern('weird key:a*b')).toBe('weird key:a\\*b:*')
  })
})

describe('createCursorStack', () => {
  it('starts at the beginning of the iteration', () => {
    const stack = createCursorStack()
    expect(stack.current()).toBe('0')
    expect(stack.canGoBack()).toBe(false)
    expect(stack.pageNumber()).toBe(1)
  })

  it('walks forward and back', () => {
    const stack = createCursorStack()
    stack.push('17408')
    expect(stack.current()).toBe('17408')
    expect(stack.pageNumber()).toBe(2)
    stack.push('35072')
    expect(stack.pageNumber()).toBe(3)

    expect(stack.back()).toBe('17408')
    expect(stack.back()).toBe('0')
    // Cannot go back past the first page.
    expect(stack.back()).toBe('0')
    expect(stack.canGoBack()).toBe(false)
  })

  it('keeps cursors as strings', () => {
    // Redis cursors are u64 and exceed Number.MAX_SAFE_INTEGER; parsing one
    // as a number would corrupt it and silently restart the iteration.
    const stack = createCursorStack()
    stack.push(12345678901234567890n.toString())
    expect(stack.current()).toBe('12345678901234567890')
    expect(typeof stack.current()).toBe('string')
  })

  it('resets to the start', () => {
    const stack = createCursorStack()
    stack.push('999')
    stack.reset()
    expect(stack.current()).toBe('0')
    expect(stack.canGoBack()).toBe(false)
  })
})

describe('formatTtl', () => {
  it('renders Redis sentinels as words, not durations', () => {
    expect(formatTtl(-1)).toBe('no expiry')
    expect(formatTtl(-2)).toBe('expired')
    expect(formatTtl(null)).toBe('no expiry')
  })

  it('renders durations', () => {
    expect(formatTtl(500)).toBe('500ms')
    expect(formatTtl(5000)).toBe('5s')
    expect(formatTtl(3_725_000)).toBe('1h 2m')
    expect(formatTtl(90_000)).toBe('1m 30s')
    expect(formatTtl(172_800_000)).toBe('2d')
  })
})

describe('formatKeyKind / isViewableKind', () => {
  it('names the types', () => {
    expect(formatKeyKind('zset')).toBe('Sorted set')
    expect(formatKeyKind('hash')).toBe('Hash')
    expect(formatKeyKind('none')).toBe('Gone')
    expect(formatKeyKind('ReJSON-RL')).toBe('ReJSON-RL')
    expect(formatKeyKind(undefined)).toBe('Unknown')
  })

  it('knows which types have a viewer', () => {
    expect(isViewableKind('stream')).toBe(true)
    // A module type has no viewer, and must not be rendered as if it did.
    expect(isViewableKind('ReJSON-RL')).toBe(false)
    expect(isViewableKind('none')).toBe(false)
  })
})

describe('summarizeScan', () => {
  it('never claims completeness while the cursor is open', () => {
    expect(summarizeScan(400, 12481, false)).toBe('400 of ~12,481')
    expect(summarizeScan(400, 0, false)).toBe('400 so far')
  })

  it('drops the estimate once the iteration finishes', () => {
    expect(summarizeScan(12481, 12481, true)).toBe('12,481 keys')
    expect(summarizeScan(1, 1, true)).toBe('1 key')
  })
})
