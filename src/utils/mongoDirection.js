/**
 * Which MongoDB connection each role in the panel refers to.
 *
 * Sync moves data between two connections, so a direction is meaningful there.
 * Dump and restore do not: each involves one connection plus a file, and both
 * act on the connection whose tree is selectable — the source side. Keeping
 * that rule here, rather than as ternaries spread through the component, is
 * what stops the roles disagreeing: a host with only a remote connection once
 * resolved its restore target to the empty local URI, which disabled restore
 * entirely.
 *
 * Pure so it can be tested; MongodbPanel.vue itself cannot be, because
 * @vue/test-utils is not installed.
 */

/**
 * `{ source, dest }`, each `'remote'` or `'local'`.
 *
 * With no local connection there is no other side, so every role is the remote
 * one. The direction toggle is hidden in that case and cannot be flipped.
 */
export function resolveSides(hasLocal, isRemoteToLocal) {
  if (!hasLocal) return { source: 'remote', dest: 'remote' }
  return isRemoteToLocal
    ? { source: 'remote', dest: 'local' }
    : { source: 'local', dest: 'remote' }
}

/** Display name for a side, for button labels. */
export function sideLabel(side) {
  return side === 'local' ? 'Local' : 'Remote'
}
