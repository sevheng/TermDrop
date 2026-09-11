import {
  isPermissionGranted,
  requestPermission,
  sendNotification,
} from '@tauri-apps/plugin-notification'

/**
 * Desktop notification for a long-running operation that has finished.
 *
 * The toast that already accompanies these only works if the user is looking
 * at the app -- and a backup big enough to be worth announcing is exactly the
 * one they switched away from. So this fires only when the window is *not*
 * focused, and the toast covers the case where it is. Firing both would just
 * say the same thing twice.
 *
 * Never throws: a refused or unavailable notification must not turn a backup
 * that actually succeeded into a visible failure. Some platforms have no
 * notification daemon at all (WSLg among them), and the plugin call rejects
 * there.
 */
export async function notifyIfUnfocused(title, body) {
  try {
    if (document.hasFocus()) return

    let granted = await isPermissionGranted()
    if (!granted) granted = (await requestPermission()) === 'granted'
    if (!granted) return

    sendNotification({ title, body })
  } catch (err) {
    console.warn('[notify] could not post notification:', err)
  }
}
