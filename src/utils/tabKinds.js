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
  VNC: 'vnc',
  // A shell on this machine. It renders in the same TerminalTab as SSH and
  // takes commands, but has no host behind it, so no SFTP/Docker/Security.
  LOCAL: 'local',
}

const KINDS = {
  [TAB_KIND.SSH]: { hasRightPanel: true, canReceiveCommand: true },
  [TAB_KIND.MONGODB]: { hasRightPanel: false, canReceiveCommand: false },
  [TAB_KIND.REDIS]: { hasRightPanel: false, canReceiveCommand: false },
  // A VNC tab takes every keystroke for the remote desktop, so it must never
  // be offered as somewhere to type a command either.
  [TAB_KIND.VNC]: { hasRightPanel: false, canReceiveCommand: false },
  [TAB_KIND.LOCAL]: { hasRightPanel: false, canReceiveCommand: true },
}

/** A tab's kind, defaulting to `ssh` for tabs that carry no type. */
export function tabKind(tab) {
  const type = tab?.type
  return type && KINDS[type] ? type : TAB_KIND.SSH
}

export function isSshTab(tab) {
  return tabKind(tab) === TAB_KIND.SSH
}

/** A tab rendered by TerminalTab: an SSH shell or a local one. */
export function isTerminalTab(tab) {
  const kind = tabKind(tab)
  return kind === TAB_KIND.SSH || kind === TAB_KIND.LOCAL
}

/**
 * The name for a new local tab: the shell's label (`Local` for the default),
 * then `zsh 2`, `zsh 3`, taking the lowest number no open local tab is using.
 */
export function nextLocalTabName(tabs, base = 'Local') {
  const used = new Set((tabs || []).filter(t => tabKind(t) === TAB_KIND.LOCAL).map(t => t.name))
  if (!used.has(base)) return base
  let n = 2
  while (used.has(`${base} ${n}`)) n++
  return `${base} ${n}`
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
  return isTerminalTab(tab) ? 'disconnect' : 'closeServiceTab'
}
