/**
 * Recognising the backend's host-key refusals.
 *
 * `ssh_connect` returns a plain string error, so structure travels inside it —
 * the same shape `secretPrompt.js` uses for the keyring retry. Unlike that
 * one, this payload decides whether the user sees a security dialog, so it is
 * matched strictly.
 *
 * Pure, so it can be tested without mounting anything.
 */

/** Must appear at the very start of the error. See `isTrustError`. */
export const TRUST_PREFIX = 'TERMDROP_TRUST:'

/**
 * Whether an error is a host-key refusal.
 *
 * Deliberately `startsWith`, not `includes`. A hostname, a server banner or a
 * remote path could contain the sentinel, and `includes` would let attacker-
 * adjacent text conjure a "trust this host" dialog for a key the backend never
 * verified. `isMissingKeyringPassword` gets away with `includes` because
 * nothing it matches is attacker-controlled; this is not that.
 */
export function isTrustError(err) {
  return String(err).startsWith(TRUST_PREFIX)
}

/**
 * Parse a refusal, or return null if it is not one.
 *
 * Returns `{ kind, host, port, fingerprint, keyType, message }`, where `kind`
 * is `'unknown'` (ask the user once) or `'refused'` (show the alarm; there is
 * no way through).
 */
export function parseTrustError(err) {
  const text = String(err)
  if (!isTrustError(text)) return null

  let payload
  try {
    payload = JSON.parse(text.slice(TRUST_PREFIX.length))
  } catch {
    // A malformed payload must not become a dialog offering to trust
    // something we cannot describe.
    return null
  }

  if (payload?.kind !== 'unknown' && payload?.kind !== 'refused') return null
  if (typeof payload.fingerprint !== 'string' || !payload.fingerprint) return null

  return payload
}

/**
 * Whether to ask the user about this host.
 *
 * `alreadyAsked` guards the loop: if a fingerprint was just accepted and the
 * backend still reports the host unknown, the key changed between attempts and
 * asking again would walk the user into accepting a different server than the
 * one they were shown.
 */
export function shouldAskToTrust(err, alreadyAsked) {
  const trust = parseTrustError(err)
  return !alreadyAsked && trust?.kind === 'unknown' ? trust : null
}
