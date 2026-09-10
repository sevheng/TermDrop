/**
 * Guards for text this app types into a live SSH shell.
 *
 * The security panel can put a remediation command at the user's prompt. That
 * shell is frequently root-capable, so the one thing that must never happen is
 * the app pressing Enter on the user's behalf. Two rules follow:
 *
 *  1. The text is written without a trailing newline. The user reads it and
 *     submits it themselves. `stripSubmit` enforces that.
 *  2. Nothing containing a control character may be inserted at all. A newline
 *     in the middle would submit the part before it, and the escape character
 *     can drive the terminal's own control sequences.
 *
 * The backend already restricts remediation commands to its own literals, so
 * this is a second, independent check at the boundary that writes to the shell.
 */

import { acceptsCommands } from './tabKinds.js'

/** Longer than this is not reviewable at a glance, so it is not offered. */
export const MAX_INSERT_LENGTH = 512

/**
 * Matches C0, DEL and C1: newline, carriage return, tab, escape, and friends.
 * Matching control characters is the entire purpose of this module, so the
 * rule that normally flags them in a pattern does not apply here.
 */
// eslint-disable-next-line no-control-regex
const CONTROL_CHARS = /[\x00-\x1F\x7F-\x9F]/

/**
 * Whether `text` is safe to type at a shell prompt for the user to review.
 * @param {unknown} text
 * @returns {boolean}
 */
export function isInsertableCommand(text) {
  if (typeof text !== 'string') return false
  // Tested before trimming, on purpose. Trimming a trailing newline away and
  // then calling the result safe would accept exactly the input this guard
  // exists to reject.
  if (CONTROL_CHARS.test(text)) return false
  const trimmed = text.trim()
  if (!trimmed) return false
  return trimmed.length <= MAX_INSERT_LENGTH
}

/**
 * The exact bytes to write. Never ends in a newline: the command lands at the
 * prompt with the cursor after it, and nothing runs until the user says so.
 * @param {string} text
 * @returns {string}
 */
export function stripSubmit(text) {
  return String(text).trim()
}

/**
 * Whether a tab can accept typed text. MongoDB tabs have no shell, and a
 * disconnected tab has nothing to write to.
 * @param {{type?: string, connected?: boolean}|null|undefined} tab
 * @returns {boolean}
 */
export function canReceiveCommand(tab) {
  if (!tab) return false
  // Asks the kind table rather than naming one type: a datastore tab has no
  // shell, and typing a remediation command into one must stay impossible as
  // kinds are added.
  if (!acceptsCommands(tab)) return false
  return tab.connected === true
}
