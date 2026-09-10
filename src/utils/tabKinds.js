/**
 * What kind of thing a tab holds.
 *
 * SSH tabs predate the `type` field and carry none, so `undefined` is `ssh`.
 * Before this table the type was compared against the string `'mongodb'` in
 * seven places, which was survivable with two kinds and is not with three:
 * every `!== 'mongodb'` check silently meant "is an SSH tab", so adding Redis
 * would have put the SFTP/Docker/Security panel beside a Redis tab and let the
 * security audit type a remediation command into it.
 */

export const TAB_KIND = {
  SSH: 'ssh',
  MONGODB: 'mongodb',
  REDIS: 'redis',
}

const KINDS = {
  [TAB_KIND.SSH]: { hasRightPanel: true, canReceiveCommand: true },
  [TAB_KIND.MONGODB]: { hasRightPanel: false, canReceiveCommand: false },
  [TAB_KIND.REDIS]: { hasRightPanel: false, canReceiveCommand: false },
}

/** A tab's kind, defaulting to `ssh` for tabs that carry no type. */
export function tabKind(tab) {
  const type = tab?.type
  return type && KINDS[type] ? type : TAB_KIND.SSH
}

export function isSshTab(tab) {
  return tabKind(tab) === TAB_KIND.SSH
}

export function tabsOfKind(tabs, kind) {
  return (tabs || []).filter(t => tabKind(t) === kind)
}

/** Only a shell tab has SFTP / Docker / Security / Tunnels beside it. */
export function hasRightPanel(tab) {
  return !!tab && KINDS[tabKind(tab)].hasRightPanel
}

/**
 * Whether a panel may type a command into this tab.
 *
 * Connectedness is the caller's business; this answers only "is this the kind
 * of tab that has a shell at all".
 */
export function acceptsCommands(tab) {
  return !!tab && KINDS[tabKind(tab)].canReceiveCommand
}

/** Which store action closes this tab. */
export function closeActionFor(tab) {
  return isSshTab(tab) ? 'disconnect' : 'closeServiceTab'
}
