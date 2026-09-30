/**
 * Choices for "go through this SSH host" pickers: the Redis tunnel and an SSH
 * host's jump host. Pure, so the filtering rules are tested without mounting.
 */
import { hostKind, HOST_KIND } from './hostKind.js'

/**
 * `SelectMenu` options: "Connect directly" first, then each host with its
 * address as a hint, so two hosts sharing a name are still tellable apart.
 */
export function sshHostOptions(hosts) {
  return [
    { value: '', label: 'Connect directly' },
    ...hosts.map(h => ({ value: h.id, label: h.name, hint: `${h.username}@${h.host}` })),
  ]
}

/**
 * Which hosts can be the jump host for the host with id `selfId` (null while
 * adding one).
 *
 * SSH hosts only, never the host itself, and never one that has a jump host of
 * its own: one hop is all the backend supports, and offering a choice it will
 * refuse at connect time only moves the error somewhere less obvious.
 */
export function jumpHostCandidates(hosts, selfId) {
  return hosts.filter(
    h => hostKind(h) === HOST_KIND.SSH && h.id !== selfId && h.jump_host_id == null,
  )
}
