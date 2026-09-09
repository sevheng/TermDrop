import { describe, it, expect } from 'vitest'
import { computeNetTotals, EMPTY_STATUS } from '../useHostStatusPolling.js'

describe('computeNetTotals', () => {
  it('sums rx and tx across interfaces and skips loopback', () => {
    const netdev = 'lo: 999 999\neth0: 100 20\nwlan0: 50 5\n'
    expect(computeNetTotals(netdev)).toEqual({ rx: 150, tx: 25 })
  })

  it('ignores short or malformed lines', () => {
    expect(computeNetTotals('eth0: 10\n\nbad line here x\n')).toEqual({ rx: 0, tx: 0 })
    expect(computeNetTotals('eth0: abc def')).toEqual({ rx: 0, tx: 0 })
    expect(computeNetTotals('')).toEqual({ rx: 0, tx: 0 })
  })
})

describe('EMPTY_STATUS', () => {
  it('has every status-bar field blank', () => {
    expect(Object.values(EMPTY_STATUS).every((v) => v === '')).toBe(true)
    expect(Object.keys(EMPTY_STATUS).sort()).toEqual(
      ['cores', 'disk', 'load', 'netDown', 'netUp', 'os', 'ram', 'uptime'],
    )
  })
})
