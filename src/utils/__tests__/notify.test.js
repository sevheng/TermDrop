import { describe, it, expect, vi, beforeEach } from 'vitest'

const granted = vi.fn()
const request = vi.fn()
const send = vi.fn()

vi.mock('@tauri-apps/plugin-notification', () => ({
  isPermissionGranted: (...a) => granted(...a),
  requestPermission: (...a) => request(...a),
  sendNotification: (...a) => send(...a),
}))

import { notifyIfUnfocused } from '../notify.js'

/** jsdom reports the document as focused by default. */
function setFocused(value) {
  vi.spyOn(document, 'hasFocus').mockReturnValue(value)
}

describe('notifyIfUnfocused', () => {
  beforeEach(() => {
    vi.restoreAllMocks()
    granted.mockReset().mockResolvedValue(true)
    request.mockReset().mockResolvedValue('granted')
    send.mockReset()
  })

  it('says nothing when the user is already looking at the app', async () => {
    // The toast alongside these already covers the focused case; firing both
    // would report the same thing twice.
    setFocused(true)
    await notifyIfUnfocused('Backup complete', 'testdb')
    expect(send).not.toHaveBeenCalled()
  })

  it('notifies when the window is in the background', async () => {
    setFocused(false)
    await notifyIfUnfocused('Backup complete', 'testdb: logs')
    expect(send).toHaveBeenCalledWith({ title: 'Backup complete', body: 'testdb: logs' })
  })

  it('asks for permission only when it does not already have it', async () => {
    setFocused(false)
    await notifyIfUnfocused('t', 'b')
    expect(request).not.toHaveBeenCalled()

    granted.mockResolvedValue(false)
    await notifyIfUnfocused('t', 'b')
    expect(request).toHaveBeenCalled()
    expect(send).toHaveBeenCalledTimes(2)
  })

  it('stays silent when permission is refused', async () => {
    setFocused(false)
    granted.mockResolvedValue(false)
    request.mockResolvedValue('denied')
    await notifyIfUnfocused('t', 'b')
    expect(send).not.toHaveBeenCalled()
  })

  it('never throws when the platform has no notification daemon', async () => {
    // WSLg among others. A backup that actually succeeded must not surface as
    // a failure because the notification could not be delivered.
    setFocused(false)
    granted.mockRejectedValue(new Error('no notification daemon'))
    vi.spyOn(console, 'warn').mockImplementation(() => {})
    await expect(notifyIfUnfocused('t', 'b')).resolves.toBeUndefined()

    granted.mockResolvedValue(true)
    send.mockImplementation(() => {
      throw new Error('dbus unavailable')
    })
    await expect(notifyIfUnfocused('t', 'b')).resolves.toBeUndefined()
  })
})
