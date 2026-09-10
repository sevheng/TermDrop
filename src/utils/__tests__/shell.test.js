import { describe, it, expect } from 'vitest'
import { shellEscape } from '../shell.js'

describe('shellEscape', () => {
  it('returns empty quotes for falsy input', () => {
    expect(shellEscape('')).toBe("''")
    expect(shellEscape(null)).toBe("''")
    expect(shellEscape(undefined)).toBe("''")
  })

  it('leaves safe characters untouched', () => {
    expect(shellEscape('abc-1.2_x/y:z@w~')).toBe('abc-1.2_x/y:z@w~')
  })

  it('single-quotes anything else', () => {
    expect(shellEscape('hello world')).toBe("'hello world'")
    expect(shellEscape('a;b')).toBe("'a;b'")
    expect(shellEscape('$HOME')).toBe("'$HOME'")
  })

  it('escapes embedded single quotes', () => {
    expect(shellEscape("it's")).toBe("'it'\"'\"'s'")
  })
})
