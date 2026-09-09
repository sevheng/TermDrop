/**
 * Quote a string for use as a single POSIX shell argument.
 * Safe characters pass through untouched; anything else is single-quoted.
 */
export function shellEscape(s) {
  if (!s) return "''"
  if (/^[a-zA-Z0-9._~\-/:@]+$/.test(s)) return s
  return "'" + s.replace(/'/g, "'\"'\"'") + "'"
}
