import { describe, it, expect, vi, afterEach } from 'vitest'
import { defaultBackupName, summarizeOp, restoreWarning } from '../redisBackup.js'

afterEach(() => vi.useRealTimers())

describe('defaultBackupName', () => {
  it('names the connection, the database and the day', () => {
    vi.useFakeTimers()
    vi.setSystemTime(new Date(2026, 8, 10, 12))
    expect(defaultBackupName('Production Cache', 0, '')).toBe(
      'production-cache-db0-2026-09-10.tdredis',
    )
  })

  it('includes the pattern, so a folder of backups is readable', () => {
    vi.useFakeTimers()
    vi.setSystemTime(new Date(2026, 8, 10, 12))
    expect(defaultBackupName('cache', 3, 'user:*')).toBe('cache-db3-user_-2026-09-10.tdredis')
  })

  it('survives a nameless or oddly named host', () => {
    vi.useFakeTimers()
    vi.setSystemTime(new Date(2026, 8, 10, 12))
    expect(defaultBackupName('', 0, '')).toBe('redis-db0-2026-09-10.tdredis')
    expect(defaultBackupName('!!!', 0, '')).toBe('redis-db0-2026-09-10.tdredis')
  })
})

describe('summarizeOp', () => {
  const base = {
    processed: 100,
    written: 100,
    skipped_existing: 0,
    skipped_big: [],
    vanished: 0,
    failed: [],
    elapsed_ms: 10,
  }

  it('reports the plain success case', () => {
    expect(summarizeOp(base, 'Restored')).toEqual({
      message: 'Restored 100 keys',
      type: 'success',
    })
  })

  it('never hides skipped keys behind a success message', () => {
    // A restore that silently skipped 12,000 existing keys "succeeded";
    // saying only that would be misleading.
    const { message } = summarizeOp({ ...base, written: 5, skipped_existing: 12000 }, 'Restored')
    expect(message).toContain('12,000 already existed')
  })

  it('counts oversized and expired keys', () => {
    const { message } = summarizeOp(
      { ...base, skipped_big: [{ key: 'big', bytes: 1 }], vanished: 3 },
      'Backed up',
    )
    expect(message).toContain('1 too large')
    expect(message).toContain('3 expired while running')
  })

  it('is a warning, not a success, when anything failed', () => {
    const result = summarizeOp({ ...base, failed: [{ key: 'k', reason: 'nope' }] }, 'Restored')
    expect(result.type).toBe('warning')
    expect(result.message).toContain('1 failed')
  })

  it('does not throw on a missing summary', () => {
    expect(summarizeOp(null, 'Restored').type).toBe('success')
  })
})

describe('restoreWarning', () => {
  it('mentions a version difference', () => {
    const warning = restoreWarning({ redis_version: '7.2.4' }, { version: '7.4.0' })
    expect(warning).toContain('7.2.4')
    expect(warning).toContain('7.4.0')
  })

  it('says nothing when the versions match', () => {
    expect(restoreWarning({ redis_version: '7.2.4' }, { version: '7.2.4' })).toBe('')
  })

  it('says nothing without both sides', () => {
    expect(restoreWarning(null, { version: '7.2.4' })).toBe('')
    expect(restoreWarning({ redis_version: '7.2.4' }, null)).toBe('')
  })
})
