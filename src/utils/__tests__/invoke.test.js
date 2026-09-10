import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest'

const tauriInvoke = vi.fn()
vi.mock('@tauri-apps/api/core', () => ({ invoke: (...a) => tauriInvoke(...a) }))

import { invoke, invokeWithSlowWarning } from '../invoke.js'

describe('invoke utils', () => {
  let toasts
  const onToast = (e) => toasts.push(e.detail)

  beforeEach(() => {
    toasts = []
    tauriInvoke.mockReset()
    window.addEventListener('app-toast', onToast)
    vi.useFakeTimers()
  })
  afterEach(() => {
    window.removeEventListener('app-toast', onToast)
    vi.useRealTimers()
  })

  it('invoke is the raw Tauri invoke', async () => {
    tauriInvoke.mockResolvedValue(42)
    await expect(invoke('cmd', { a: 1 })).resolves.toBe(42)
    expect(tauriInvoke).toHaveBeenCalledWith('cmd', { a: 1 })
    expect(toasts).toEqual([])
  })

  it('invokeWithSlowWarning passes through fast calls silently', async () => {
    tauriInvoke.mockResolvedValue('ok')
    const result = invokeWithSlowWarning('get_hosts')
    vi.advanceTimersByTime(9_999)
    await expect(result).resolves.toBe('ok')
    vi.advanceTimersByTime(10_000)
    expect(toasts).toEqual([])
  })

  it('invokeWithSlowWarning warns once after 10s and still resolves', async () => {
    let resolve
    tauriInvoke.mockReturnValue(new Promise((r) => (resolve = r)))
    const result = invokeWithSlowWarning('slow_cmd')
    vi.advanceTimersByTime(10_000)
    expect(toasts).toEqual([{ message: 'slow_cmd is taking longer than expected...', type: 'warning' }])
    resolve('done')
    await expect(result).resolves.toBe('done')
    expect(toasts).toHaveLength(1)
  })
})
