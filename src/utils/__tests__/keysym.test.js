import { describe, it, expect } from 'vitest'
import {
  keysymFromEvent,
  isPhantomControlForAltGr,
  MODIFIER_KEYSYMS,
  CTRL_ALT_DELETE,
} from '../keysym.js'

/** A keyboard event is only ever read for these two fields. */
const ev = (key, code) => ({ key, code })
const sym = (key, code) => keysymFromEvent(ev(key, code))

describe('keysymFromEvent', () => {
  it('maps ASCII characters to their own code points', () => {
    // Latin-1 keysyms *are* the code points, which is what keeps this module
    // from needing a table of them.
    expect(sym('a', 'KeyA')).toEqual({ keysym: 0x61, ok: true })
    expect(sym('A', 'KeyA')).toEqual({ keysym: 0x41, ok: true })
    expect(sym('1', 'Digit1')).toEqual({ keysym: 0x31, ok: true })
    expect(sym('!', 'Digit1')).toEqual({ keysym: 0x21, ok: true })
    expect(sym(' ', 'Space')).toEqual({ keysym: 0x20, ok: true })
  })

  it('reads the character from key, not the letter printed on the key', () => {
    // On a French layout the key labelled Q types "a". Reading `code` would
    // send "q" and make the remote unusable for anyone outside the US layout.
    expect(sym('a', 'KeyQ')).toEqual({ keysym: 0x61, ok: true })
  })

  it('maps the rest of Latin-1 directly too', () => {
    expect(sym('é', 'KeyE')).toEqual({ keysym: 0xe9, ok: true })
    expect(sym('ÿ', 'KeyY')).toEqual({ keysym: 0xff, ok: true })
  })

  it('maps everything above Latin-1 into the Unicode keysym range', () => {
    // The second of the two rules that replace a Unicode table.
    expect(sym('€', 'KeyE')).toEqual({ keysym: 0x010020ac, ok: true })
    expect(sym('中', 'KeyA')).toEqual({ keysym: 0x01004e2d, ok: true })
    expect(sym('й', 'KeyQ')).toEqual({ keysym: 0x01000439, ok: true })
  })

  it('handles an astral character given as a surrogate pair', () => {
    // "😀" is two UTF-16 units but one code point; length checks
    // alone would misread it as a named key.
    expect(sym('\u{1f600}', 'KeyA')).toEqual({ keysym: 0x0101f600, ok: true })
  })

  it('maps named keys from code', () => {
    expect(sym('Enter', 'Enter')).toEqual({ keysym: 0xff0d, ok: true })
    expect(sym('Escape', 'Escape')).toEqual({ keysym: 0xff1b, ok: true })
    expect(sym('Backspace', 'Backspace')).toEqual({ keysym: 0xff08, ok: true })
    expect(sym('ArrowUp', 'ArrowUp')).toEqual({ keysym: 0xff52, ok: true })
    expect(sym('Delete', 'Delete')).toEqual({ keysym: 0xffff, ok: true })
    expect(sym('Home', 'Home')).toEqual({ keysym: 0xff50, ok: true })
  })

  it('numbers the function keys contiguously from F1', () => {
    expect(sym('F1', 'F1')).toEqual({ keysym: 0xffbe, ok: true })
    expect(sym('F11', 'F11')).toEqual({ keysym: 0xffc8, ok: true })
    expect(sym('F12', 'F12')).toEqual({ keysym: 0xffc9, ok: true })
  })

  it('tells the two sides of a modifier apart', () => {
    // `event.key` reports both as "Shift"; only `code` distinguishes them,
    // which is the whole reason the table is keyed on code.
    expect(sym('Shift', 'ShiftLeft').keysym).toBe(0xffe1)
    expect(sym('Shift', 'ShiftRight').keysym).toBe(0xffe2)
    expect(sym('Control', 'ControlLeft').keysym).toBe(0xffe3)
    expect(sym('Control', 'ControlRight').keysym).toBe(0xffe4)
    expect(sym('Meta', 'MetaLeft').keysym).toBe(0xffeb)
  })

  it('separates the keypad from the main keyboard', () => {
    // Remote software does tell these apart -- a spreadsheet's keypad Enter
    // behaves differently from the main one.
    expect(sym('Enter', 'NumpadEnter').keysym).toBe(0xff8d)
    expect(sym('Enter', 'Enter').keysym).toBe(0xff0d)
    expect(sym('.', 'NumpadDecimal').keysym).toBe(0xffae)
    expect(sym('5', 'Numpad5').keysym).toBe(0xffb5)
    // ...and the main row's 5 is still the character.
    expect(sym('5', 'Digit5').keysym).toBe(0x35)
  })

  it('refuses to guess at a key it has no symbol for', () => {
    // Sending something here would be a garbage keysym, which is worse than
    // sending nothing at all.
    expect(sym('Dead', 'Backquote')).toEqual({ ok: false })
    expect(sym('Unidentified', 'Unidentified')).toEqual({ ok: false })
    expect(sym('AudioVolumeUp', 'AudioVolumeUp')).toEqual({ ok: false })
    expect(sym('', '')).toEqual({ ok: false })
    expect(keysymFromEvent(undefined)).toEqual({ ok: false })
  })

  it('exports the modifiers it can emit, so none can be left stuck', () => {
    // Anything the table can send down must appear here, or a key held while
    // the window loses focus stays held on the remote forever.
    for (const code of [
      'ShiftLeft', 'ShiftRight', 'ControlLeft', 'ControlRight',
      'AltLeft', 'AltRight', 'MetaLeft', 'MetaRight', 'CapsLock',
    ]) {
      expect(MODIFIER_KEYSYMS).toContain(sym('x', code).keysym)
    }
  })

  it('spells Ctrl+Alt+Del out of the same keysyms the table uses', () => {
    expect(CTRL_ALT_DELETE).toEqual([
      sym('Control', 'ControlLeft').keysym,
      sym('Alt', 'AltLeft').keysym,
      sym('Delete', 'Delete').keysym,
    ])
  })
})

describe('isPhantomControlForAltGr', () => {
  it('recognises the Control that Windows fires just before AltGr', () => {
    // The pair arrives in the same tick; left alone, typing "@" on a German
    // layout reaches the remote as Ctrl+@.
    expect(isPhantomControlForAltGr(1000, 1005)).toBe(true)
    expect(isPhantomControlForAltGr(1000, 1000)).toBe(true)
  })

  it('leaves a genuinely held Control alone', () => {
    // Someone really holding Ctrl and then reaching for AltGr is far outside
    // the window, and Ctrl must keep working as Ctrl.
    expect(isPhantomControlForAltGr(1000, 1200)).toBe(false)
    expect(isPhantomControlForAltGr(null, 1005)).toBe(false)
    expect(isPhantomControlForAltGr(undefined, 1005)).toBe(false)
  })

  it('ignores a Control pressed after the AltGr', () => {
    // Negative gap: not the synthetic pair, whatever the clock says.
    expect(isPhantomControlForAltGr(1010, 1005)).toBe(false)
  })
})
