import { describe, it, expect } from 'vitest'
import {
  isInsertableCommand,
  stripSubmit,
  canReceiveCommand,
  MAX_INSERT_LENGTH,
} from '../terminalInsert.js'

describe('isInsertableCommand', () => {
  it('accepts an ordinary single-line command', () => {
    expect(isInsertableCommand('sudo ufw status verbose')).toBe(true)
    expect(isInsertableCommand("sudo grep -rniE '^ *port' /etc/ssh/sshd_config")).toBe(true)
  })

  it('rejects anything that could submit itself', () => {
    // A trailing newline would run the command; an embedded one would run the
    // first half and leave the rest sitting at the prompt.
    expect(isInsertableCommand('whoami\n')).toBe(false)
    expect(isInsertableCommand('whoami\r')).toBe(false)
    expect(isInsertableCommand('echo a\nrm -rf /')).toBe(false)
  })

  it('rejects control characters that drive the terminal itself', () => {
    expect(isInsertableCommand('echo \x1b[2J')).toBe(false)
    expect(isInsertableCommand('echo \x07')).toBe(false)
    expect(isInsertableCommand('echo \x00')).toBe(false)
    expect(isInsertableCommand('a\tb')).toBe(false)
  })

  it('rejects empty, blank, and non-string input', () => {
    for (const bad of ['', '   ', null, undefined, 42, {}, ['ls']]) {
      expect(isInsertableCommand(bad)).toBe(false)
    }
  })

  it('rejects text too long to read before pressing Enter', () => {
    expect(isInsertableCommand('a'.repeat(MAX_INSERT_LENGTH))).toBe(true)
    expect(isInsertableCommand('a'.repeat(MAX_INSERT_LENGTH + 1))).toBe(false)
  })
})

describe('stripSubmit', () => {
  it('never returns text ending in a newline', () => {
    for (const input of ['ls -la', 'ls -la\n', '  ls -la  \n', 'ls -la\r\n']) {
      const out = stripSubmit(input)
      expect(out).toBe('ls -la')
      expect(out.endsWith('\n')).toBe(false)
      expect(out.endsWith('\r')).toBe(false)
    }
  })
})

describe('canReceiveCommand', () => {
  it('accepts a connected SSH tab', () => {
    expect(canReceiveCommand({ id: 's1', connected: true })).toBe(true)
  })

  it('refuses a MongoDB tab, which has no shell', () => {
    expect(canReceiveCommand({ id: 'mongo-1', type: 'mongodb', connected: true })).toBe(false)
  })

  it('refuses a disconnected tab and a missing tab', () => {
    expect(canReceiveCommand({ id: 's1', connected: false })).toBe(false)
    expect(canReceiveCommand({ id: 's1' })).toBe(false)
    expect(canReceiveCommand(null)).toBe(false)
    expect(canReceiveCommand(undefined)).toBe(false)
  })
})
