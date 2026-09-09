import { describe, it, expect, vi, beforeEach } from 'vitest'

const pending = []
const tauriListen = vi.fn(() => new Promise((resolve) => pending.push(resolve)))
vi.mock('@tauri-apps/api/event', () => ({ listen: (...a) => tauriListen(...a) }))

import { useListenerGroup } from '../useListenerGroup.js'

describe('useListenerGroup', () => {
  beforeEach(() => {
    pending.length = 0
    tauriListen.mockClear()
  })

  it('subscribes and disposes registered listeners', async () => {
    const group = useListenerGroup()
    const unlisten = vi.fn()
    const p = group.listen('evt', () => {})
    pending[0](unlisten)
    await p
    expect(tauriListen).toHaveBeenCalledWith('evt', expect.any(Function))
    expect(unlisten).not.toHaveBeenCalled()
    group.dispose()
    expect(unlisten).toHaveBeenCalledTimes(1)
  })

  it('unlistens immediately when disposed before the subscription resolves', async () => {
    const group = useListenerGroup()
    const unlisten = vi.fn()
    const p = group.listen('evt', () => {})
    group.dispose()
    pending[0](unlisten)
    await p
    expect(unlisten).toHaveBeenCalledTimes(1)
    group.dispose()
    expect(unlisten).toHaveBeenCalledTimes(1)
  })

  it('is reusable after dispose', async () => {
    const group = useListenerGroup()
    group.dispose()
    const unlisten = vi.fn()
    const p = group.listen('evt', () => {})
    pending[0](unlisten)
    await p
    expect(unlisten).not.toHaveBeenCalled()
    group.dispose()
    expect(unlisten).toHaveBeenCalledTimes(1)
  })
})
