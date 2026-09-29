import { describe, it, expect } from 'vitest'
import { parseVncTarget, vncDisplayTarget, BASE_PORT } from '../vncTarget.js'

describe('parseVncTarget', () => {
  it('defaults a bare address to the first display', () => {
    expect(parseVncTarget('10.0.0.5')).toEqual({ host: '10.0.0.5', port: 5900 })
    expect(parseVncTarget('  kiosk.local  ')).toEqual({ host: 'kiosk.local', port: 5900 })
  })

  it('reads a small number after the colon as a display, not a port', () => {
    // People write ":1" because that is how VNC has always been written down,
    // and a field that rejects it looks broken.
    expect(parseVncTarget('host:1')).toEqual({ host: 'host', port: 5901 })
    expect(parseVncTarget('host:0')).toEqual({ host: 'host', port: 5900 })
    expect(parseVncTarget('host:99')).toEqual({ host: 'host', port: 5999 })
  })

  it('reads a large number as the port it plainly is', () => {
    // Nobody means display 5901. The two ranges cannot overlap, because the
    // smallest port anyone would type here is 5900 itself.
    expect(parseVncTarget('host:5901')).toEqual({ host: 'host', port: 5901 })
    expect(parseVncTarget('host:443')).toEqual({ host: 'host', port: 443 })
  })

  it('honours the double colon as "this really is a port"', () => {
    // RealVNC's notation, and the only way to say port 1.
    expect(parseVncTarget('host::5901')).toEqual({ host: 'host', port: 5901 })
    expect(parseVncTarget('host::1')).toEqual({ host: 'host', port: 1 })
  })

  it('takes a bracketed IPv6 address with or without a display', () => {
    expect(parseVncTarget('[::1]')).toEqual({ host: '::1', port: BASE_PORT })
    expect(parseVncTarget('[::1]:1')).toEqual({ host: '::1', port: 5901 })
    expect(parseVncTarget('[fe80::1%eth0]::5900')).toEqual({ host: 'fe80::1%eth0', port: 5900 })
  })

  it('asks for brackets rather than guessing at a bare IPv6 address', () => {
    // "::1:5901" is genuinely ambiguous, so the error says what to type.
    const result = parseVncTarget('fe80::1:5901')
    expect(result.error).toMatch(/square brackets/)
  })

  it('rejects what it cannot parse, with a reason', () => {
    expect(parseVncTarget('').error).toBeTruthy()
    expect(parseVncTarget('   ').error).toBeTruthy()
    expect(parseVncTarget(null).error).toBeTruthy()
    expect(parseVncTarget(':5901').error).toBeTruthy()
    expect(parseVncTarget('host:').error).toBeTruthy()
    expect(parseVncTarget('host:abc').error).toMatch(/not a display number or port/)
    expect(parseVncTarget('[::1').error).toMatch(/Unclosed/)
    expect(parseVncTarget('host::99999').error).toMatch(/1-65535/)
  })
})

describe('vncDisplayTarget', () => {
  it('always shows a port, never a display number', () => {
    // Showing ":1" for 5901 but ":5999" for 5999 would be a needless puzzle.
    expect(vncDisplayTarget({ vnc_host: 'h', vnc_port: 5901 })).toBe('h:5901')
    expect(vncDisplayTarget({ vnc_host: 'h', vnc_port: 5999 })).toBe('h:5999')
  })

  it('falls back to the base port when none is stored', () => {
    expect(vncDisplayTarget({ vnc_host: 'h' })).toBe('h:5900')
    expect(vncDisplayTarget({ vnc_host: 'h', vnc_port: 0 })).toBe('h:5900')
  })

  it('re-brackets an IPv6 address so the result can be parsed back', () => {
    expect(vncDisplayTarget({ vnc_host: '::1', vnc_port: 5901 })).toBe('[::1]:5901')
    expect(parseVncTarget(vncDisplayTarget({ vnc_host: '::1', vnc_port: 5901 })))
      .toEqual({ host: '::1', port: 5901 })
  })

  it('is empty for a row with no VNC settings', () => {
    expect(vncDisplayTarget({})).toBe('')
    expect(vncDisplayTarget(null)).toBe('')
  })
})
