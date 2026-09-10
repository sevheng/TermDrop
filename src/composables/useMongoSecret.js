import { withStoredSecret } from './useSecretRetry.js'

/**
 * Run a MongoDB call, prompting once for the password if none is stored.
 *
 * A thin wrapper over {@link withStoredSecret} so the Mongo call sites keep the
 * name they already use; the prompt-and-retry logic itself is shared with
 * Redis rather than copied.
 */
export function withMongoSecret(hostId, fn) {
  return withStoredSecret(
    {
      hostId,
      storeCommand: 'mongodb_store_secret',
      title: 'MongoDB password required',
      message: 'No stored password for this MongoDB connection. Enter it to continue:',
    },
    fn,
  )
}
