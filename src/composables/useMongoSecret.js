import { invoke } from '../utils/invoke.js'
import { shouldPromptForSecret } from '../utils/secretPrompt.js'
import { showPromptDialog } from './usePromptDialog.js'

/** Human wording for a side, used in the prompt. */
const SIDE_LABEL = { remote: 'remote', local: 'local' }

/**
 * Run a MongoDB call, prompting once for the password if none is stored.
 *
 * MongoDB URIs no longer carry their password, so the backend splices it in
 * from the keyring. When there is nothing stored it reports the same
 * "keyring retrieve failed" wording the SSH path uses; ask the user, store it,
 * and run the call again exactly once.
 */
export async function withMongoSecret(hostId, side, fn) {
  try {
    return await fn()
  } catch (err) {
    if (!shouldPromptForSecret(err, false)) throw err

    const password = await showPromptDialog(
      'MongoDB password required',
      `No stored password for the ${SIDE_LABEL[side] || side} MongoDB connection. Enter it to continue:`,
      '',
      'password',
    )
    if (!password) throw err

    await invoke('mongodb_store_secret', { hostId, side, password })
    return await fn()
  }
}
