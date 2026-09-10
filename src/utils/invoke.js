import { invoke as tauriInvoke } from '@tauri-apps/api/core'
import { toast } from './toast.js'

const SLOW_INVOKE_MS = 10000 // 10 seconds

/**
 * Raw Tauri invoke, re-exported so every call site imports from one place.
 * Use this for commands that legitimately run a long time (mongodb_*,
 * sftp_download_dir, docker_install) so they do not trigger the warning.
 */
export { tauriInvoke as invoke }

/**
 * Wrap Tauri invoke() with a freeze-detection timer.
 * If the call takes longer than SLOW_INVOKE_MS, a warning toast is shown.
 */
export function invokeWithSlowWarning(cmd, args = {}) {
  const start = performance.now()
  const timer = setTimeout(() => {
    toast(`${cmd} is taking longer than expected...`, 'warning')
  }, SLOW_INVOKE_MS)

  return tauriInvoke(cmd, args).finally(() => {
    clearTimeout(timer)
    const elapsed = performance.now() - start
    if (elapsed > SLOW_INVOKE_MS) {
      console.warn(`[SLOW] ${cmd} took ${elapsed.toFixed(0)}ms`, args)
    }
  })
}
