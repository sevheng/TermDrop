/**
 * Parsing and formatting a VNC server address.
 *
 * VNC has no URI form, so the host is stored as discrete columns and this is
 * the one place that turns what a person types into them. The awkward part is
 * historical: VNC addresses are conventionally written with a *display number*
 * rather than a port, where display N means port 5900 + N. People type `:1`,
 * and a field that rejects it looks broken — but `:5901` is just as common, and
 * RealVNC's `::5901` exists precisely to say "this really is a port".
 */

/** The first VNC display's port. Display N listens on BASE_PORT + N. */
export const BASE_PORT = 5900

/**
 * The largest value still read as a display number rather than a port.
 *
 * Displays run 0-99 by convention; anything above that someone typed is a port
 * they meant literally. The two ranges cannot overlap because the smallest
 * plausible port here is 5900 itself.
 */
const MAX_DISPLAY = 99

/**
 * Parse a typed address into `{ host, port }`, or `{ error }`.
 *
 * Accepts `host`, `host:5901`, `host::5901`, `host:1`, and bracketed IPv6
 * (`[::1]:1`), which is why the brackets are stripped before the last colon is
 * looked for — an unbracketed IPv6 literal is indistinguishable from a
 * host:port and is rejected rather than guessed at.
 */
export function parseVncTarget(text) {
  const raw = (text ?? '').trim()
  if (!raw) return { error: 'An address is required' }

  // Bracketed IPv6: the authority ends at the closing bracket, so any colon
  // before it belongs to the address itself.
  if (raw.startsWith('[')) {
    const close = raw.indexOf(']')
    if (close === -1) return { error: 'Unclosed [ in the IPv6 address' }
    const host = raw.slice(1, close)
    const rest = raw.slice(close + 1)
    if (!host) return { error: 'An address is required' }
    if (!rest) return { host, port: BASE_PORT }
    if (!rest.startsWith(':')) return { error: `Unexpected ${JSON.stringify(rest)} after the address` }
    return withSuffix(host, rest.slice(1))
  }

  const colon = raw.indexOf(':')
  if (colon === -1) return { host: raw, port: BASE_PORT }

  const host = raw.slice(0, colon)
  if (!host) return { error: 'An address is required' }
  const rest = raw.slice(colon + 1)

  // More than one colon left means an unbracketed IPv6 literal, except for the
  // `::port` form, which withSuffix handles.
  if (!rest.startsWith(':') && rest.includes(':')) {
    return { error: 'Put an IPv6 address in square brackets, like [::1]:1' }
  }
  return withSuffix(host, rest)
}

/** Resolve whatever followed the first colon into a port. */
function withSuffix(host, suffix) {
  // `::5901` — RealVNC's way of saying "a port, not a display".
  const explicitPort = suffix.startsWith(':')
  const digits = explicitPort ? suffix.slice(1) : suffix

  if (!digits) return { error: 'Add a display number or port after the colon' }
  // An unbracketed IPv6 address reaches here too: `fe80::1:5901` looks like
  // the `::port` form until you notice the remainder still has a colon in it.
  if (digits.includes(':')) {
    return { error: 'Put an IPv6 address in square brackets, like [::1]:1' }
  }
  if (!/^\d+$/.test(digits)) {
    return { error: `${JSON.stringify(digits)} is not a display number or port` }
  }

  const value = Number(digits)
  const port = explicitPort || value > MAX_DISPLAY ? value : BASE_PORT + value
  if (port < 1 || port > 65535) {
    return { error: `Port ${port} is outside 1-65535` }
  }
  return { host, port }
}

/**
 * Format a host row's VNC address for display.
 *
 * Always as `host:port`, never as a display number: the stored value is a port,
 * and showing `:1` for 5901 but `:5999` for 5999 would be a needless puzzle.
 */
export function vncDisplayTarget(host) {
  if (!host?.vnc_host) return ''
  const address = host.vnc_host.includes(':') ? `[${host.vnc_host}]` : host.vnc_host
  return `${address}:${host.vnc_port || BASE_PORT}`
}
