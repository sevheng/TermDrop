import { ref } from 'vue'
import { invoke } from '../utils/invoke.js'
import { withRedisSecret } from './useSecretRetry.js'
import { toast } from '../utils/toast.js'

/**
 * A Redis host's connection and what the server says about itself.
 *
 * The connection is named by host id, never by URI: the stored URI carries no
 * password, so the backend resolves the full connection string itself and the
 * credential never reaches the frontend.
 */
export function useRedisConnection(hostId) {
  const info = ref(null)
  const connecting = ref(false)
  const error = ref('')

  async function connect() {
    if (!hostId.value) return
    connecting.value = true
    error.value = ''
    try {
      info.value = await withRedisSecret(hostId.value, () =>
        invoke('redis_connect', { hostId: hostId.value }),
      )
    } catch (err) {
      // Shown as a banner with a Reconnect button rather than a toast: this is
      // the state of the panel, not a passing event, and for a tunnelled host
      // the message is the only clue about the bastion.
      error.value = String(err)
      info.value = null
    } finally {
      connecting.value = false
    }
  }

  /** Re-read INFO, so key counts follow a backup or restore. */
  async function refresh() {
    if (!hostId.value || !info.value) return
    try {
      info.value = await invoke('redis_server_info', { hostId: hostId.value })
    } catch (err) {
      toast(`Could not refresh the server info: ${err}`, 'error')
    }
  }

  function reset() {
    info.value = null
    error.value = ''
  }

  return { info, connecting, error, connect, refresh, reset }
}
