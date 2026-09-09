import { describe, it, expect, vi } from 'vitest'
import { toast } from '../toast.js'

describe('toast', () => {
  it('dispatches an app-toast event with message and type', () => {
    const handler = vi.fn()
    window.addEventListener('app-toast', handler)
    toast('Saved', 'success')
    window.removeEventListener('app-toast', handler)
    expect(handler).toHaveBeenCalledTimes(1)
    expect(handler.mock.calls[0][0].detail).toEqual({ message: 'Saved', type: 'success' })
  })

  it('requires an explicit type', () => {
    expect(() => toast('oops')).toThrow(/requires a type/)
  })
})
