/**
 * Detecting the backend's "no stored password" error.
 *
 * The SSH and MongoDB paths both report a missing secret with the wording
 * crypto.rs produces, so one detector drives both prompt-and-retry flows.
 * Pure, so it can be tested without mounting a component.
 */

/** The backend could not find a stored password for a connection. */
export function isMissingKeyringPassword(err) {
  const errStr = String(err)
  return errStr.includes('keyring retrieve failed') || errStr.includes('No matching entry')
}

/**
 * Whether to prompt the user for a password and retry.
 *
 * `alreadyRetried` guards against a loop: if a password was just supplied and
 * the backend still reports one missing, prompting again would not help.
 */
export function shouldPromptForSecret(err, alreadyRetried) {
  return !alreadyRetried && isMissingKeyringPassword(err)
}
