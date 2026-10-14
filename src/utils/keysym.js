/**
 * Browser keyboard events to X11 keysyms.
 *
 * The table below is only half the job, and the smaller half. The two rules in
 * `keysymFromEvent` cover every printable character in every language, which is
 * why there is no giant Unicode table here and must never be one:
 *
 *   - Latin-1 (U+0020-U+00FF) keysyms *are* the code points. `a` is 0x61.
 *   - Everything else is `0x01000000 + code point`, the Unicode keysym range.
 *
 * The table is for the keys that produce no character at all.
 *
 * `event.key` is used for characters and `event.code` for everything else, and
 * the split is deliberate. `key` already has the keyboard layout and the
 * modifiers applied, so it *is* the symbol the remote should receive — reading
 * `code` for a character would send the US-layout letter printed on the key
 * rather than the one the user typed. But `key` reports both shifts as
 * `"Shift"`, and only `code` distinguishes `ShiftLeft` from `ShiftRight`.
 */

/** `event.code` to keysym, for keys that produce no character. */
const BY_CODE = {
  // Editing and navigation
  Escape: 0xff1b,
  Tab: 0xff09,
  Backspace: 0xff08,
  Enter: 0xff0d,
  Insert: 0xff63,
  Delete: 0xffff,
  Home: 0xff50,
  End: 0xff57,
  PageUp: 0xff55,
  PageDown: 0xff56,
  ArrowLeft: 0xff51,
  ArrowUp: 0xff52,
  ArrowRight: 0xff53,
  ArrowDown: 0xff54,

  // Modifiers. Left and right are genuinely different keysyms, and some remote
  // software distinguishes them — which is the whole reason this table is
  // keyed on `code` rather than `key`.
  ShiftLeft: 0xffe1,
  ShiftRight: 0xffe2,
  ControlLeft: 0xffe3,
  ControlRight: 0xffe4,
  CapsLock: 0xffe5,
  AltLeft: 0xffe9,
  AltRight: 0xffea, // Alt_R. On layouts where this key is AltGr the character
  //                  path already has it applied — see isPhantomControlForAltGr.
  MetaLeft: 0xffeb,
  MetaRight: 0xffec,
  OSLeft: 0xffeb,
  OSRight: 0xffec,
  ContextMenu: 0xff67,

  // Locks and system keys
  NumLock: 0xff7f,
  ScrollLock: 0xff14,
  Pause: 0xff13,
  PrintScreen: 0xff61,

  // Numeric keypad. Distinct keysyms from the main row: remote applications
  // can and do tell them apart, and a spreadsheet's Enter behaves differently
  // from its keypad Enter.
  NumpadDivide: 0xffaf,
  NumpadMultiply: 0xffaa,
  NumpadSubtract: 0xffad,
  NumpadAdd: 0xffab,
  NumpadEnter: 0xff8d,
  NumpadDecimal: 0xffae,
  Numpad0: 0xffb0,
  Numpad1: 0xffb1,
  Numpad2: 0xffb2,
  Numpad3: 0xffb3,
  Numpad4: 0xffb4,
  Numpad5: 0xffb5,
  Numpad6: 0xffb6,
  Numpad7: 0xffb7,
  Numpad8: 0xffb8,
  Numpad9: 0xffb9,
}

// Function keys: F1-F35 run contiguously from 0xFFBE, so a loop is both
// shorter and less likely to contain a typo than 35 hand-written lines.
for (let n = 1; n <= 24; n++) {
  BY_CODE[`F${n}`] = 0xffbe + n - 1
}

/** Keysyms for the modifier keys, used to release anything left held. */
export const MODIFIER_KEYSYMS = [
  0xffe1, 0xffe2, 0xffe3, 0xffe4, 0xffe9, 0xffea, 0xffeb, 0xffec, 0xffe5,
]

/** Ctrl+Alt+Del, which no browser will ever deliver as a keydown. */
export const CTRL_ALT_DELETE = [0xffe3, 0xffe9, 0xffff]

/**
 * The keysym for a keyboard event, or `{ ok: false }` if there is none.
 *
 * `ok: false` covers dead keys mid-composition and `Unidentified`, both of
 * which would otherwise be sent as a garbage keysym.
 */
export function keysymFromEvent(event) {
  const { key, code } = event ?? {}

  // Named keys first: `key` for Enter is "Enter", which has length 5 and so
  // would fall through the character rule anyway, but Space is " " — a
  // printable character whose code is "Space", and which must go down the
  // character path to produce 0x20.
  if (key !== ' ' && code && Object.prototype.hasOwnProperty.call(BY_CODE, code)) {
    return { keysym: BY_CODE[code], ok: true }
  }

  if (typeof key !== 'string' || key.length === 0) return { ok: false }

  // A named key we have no entry for ("Dead", "Unidentified", "F25",
  // "AudioVolumeUp"). Sending anything here would be a guess.
  if (key.length > 1 && !isSingleCodePoint(key)) return { ok: false }

  const codePoint = key.codePointAt(0)

  // Latin-1 keysyms are the code points themselves.
  if (codePoint >= 0x20 && codePoint <= 0xff) return { keysym: codePoint, ok: true }

  // Everything else — Greek, Cyrillic, CJK, emoji — goes in the Unicode range.
  // One rule, no table, every language.
  return { keysym: 0x01000000 + codePoint, ok: true }
}

/** Whether a string is one code point written as a surrogate pair. */
function isSingleCodePoint(text) {
  return Array.from(text).length === 1
}

/** How close together the AltGr pair must arrive to be treated as one key. */
export const ALTGR_WINDOW_MS = 20

/**
 * Whether a Control that was just sent turns out to have been AltGr.
 *
 * On a Windows non-US layout, AltGr arrives as **`ControlLeft` first, then
 * `AltRight`**. Nothing about the Control event says it is coming, so it has
 * already been sent by the time we can tell — which is why this is asked when
 * the AltRight arrives, and why the caller's response is to send a Control
 * *release* rather than to suppress anything.
 *
 * Left alone, typing `@` on a German keyboard reaches the remote as Ctrl+@.
 * Releasing costs nothing, because the character path reads `event.key`, which
 * already has AltGr applied.
 *
 * `controlDownAt` is `null` when no Control is being held.
 */
export function isPhantomControlForAltGr(controlDownAt, altGrDownAt) {
  if (controlDownAt === null || controlDownAt === undefined) return false
  const gap = altGrDownAt - controlDownAt
  return gap >= 0 && gap < ALTGR_WINDOW_MS
}
