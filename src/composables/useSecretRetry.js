import { invoke } from '../utils/invoke.js'
import { shouldPromptForSecret } from '../utils/secretPrompt.js'
import { showPromptDialog } from './usePromptDialog.js'

/**
 * Run a datastore call, prompting once for the password if none is stored.
 *
 * Neither MongoDB nor Redis URIs carry their password any more: the backend
 * splices it in from the keyring, and when there is nothing stored it reports
 * the same "keyring retrieve failed" wording the SSH path uses. Ask the user,
 * store it, and run the call again exactly once — retrying more than once
 * would loop against a genuinely wrong password.
 */
export async function withStoredSecret({ hostId, storeCommand, title, message }, fn) {
  try {
    return await fn()
  } catch (err) {
    if (!shouldPromptForSecret(err, false)) throw err

    const password = await showPromptDialog(title, message, '', 'password')
    if (!password) throw err

    await invoke(storeCommand, { hostId, password })
    return await fn()
  }
}

/** Redis flavour of {@link withStoredSecret}. */
export function withRedisSecret(hostId, fn) {
  return withStoredSecret(
    {
      hostId,
      storeCommand: 'redis_store_secret',
      title: 'Redis password required',
      message: 'No stored password for this Redis connection. Enter it to continue:',
    },
    fn,
  )
}
