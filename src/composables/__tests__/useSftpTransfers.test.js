import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest'
import { useSftpTransfers } from '../useSftpTransfers.js'

describe('useSftpTransfers', () => {
  beforeEach(() => vi.useFakeTimers())
  afterEach(() => vi.useRealTimers())

  it('adds a row on first progress and computes speed on the next', () => {
    const { transfers, handleProgress } = useSftpTransfers()
    vi.setSystemTime(1000)
    handleProgress({ file: '/a/b.txt', bytes_transferred: 100, total_bytes: 1000 })
    expect(transfers.value).toHaveLength(1)
    expect(transfers.value[0]).toMatchObject({ file: '/a/b.txt', fileName: 'b.txt', bytes: 100, total: 1000, speed: 0, done: false })

    vi.setSystemTime(2000)
    handleProgress({ file: '/a/b.txt', bytes_transferred: 600, total_bytes: 1000 })
    expect(transfers.value[0]).toMatchObject({ bytes: 600, speed: 500, done: false })
  })

  it('marks done rows and removes them after 3s', () => {
    const { transfers, handleProgress } = useSftpTransfers()
    handleProgress({ file: '/x', bytes_transferred: 5, total_bytes: 5 })
    expect(transfers.value[0].done).toBe(true)
    vi.advanceTimersByTime(2999)
    expect(transfers.value).toHaveLength(1)
    vi.advanceTimersByTime(1)
    expect(transfers.value).toHaveLength(0)
  })

  it('treats a zero-byte total as done', () => {
    const { transfers, handleProgress } = useSftpTransfers()
    handleProgress({ file: '/empty', bytes_transferred: 0, total_bytes: 0 })
    expect(transfers.value[0].done).toBe(true)
  })

  it('tracks folder downloads as indeterminate rows until finished', () => {
    const { transfers, beginFolderTransfer, finishFolderTransfer } = useSftpTransfers()
    const file = { name: 'logs', path: '/var/logs' }
    const key = beginFolderTransfer(file)
    expect(key).toBe('folder:/var/logs')
    expect(transfers.value[0]).toMatchObject({ fileName: '📁 logs', total: 0, done: false })
    finishFolderTransfer(key, file, '(saved)')
    expect(transfers.value[0]).toMatchObject({ fileName: '📁 logs (saved)', done: true })
    vi.advanceTimersByTime(3000)
    expect(transfers.value).toHaveLength(0)
  })

  it('clearTimers cancels pending removals', () => {
    const { transfers, handleProgress, clearTimers } = useSftpTransfers()
    handleProgress({ file: '/x', bytes_transferred: 5, total_bytes: 5 })
    clearTimers()
    vi.advanceTimersByTime(5000)
    expect(transfers.value).toHaveLength(1)
  })
})
