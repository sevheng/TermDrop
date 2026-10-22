//! What kind of thing a tab holds. Port of `src/utils/tabKinds.js`.
//!
//! In the Vue app the type was compared against the string `'mongodb'` in
//! seven places, and every `!== 'mongodb'` silently meant "is an SSH tab".
//! Adding Redis would have put the SFTP/Docker/Security panel beside a Redis
//! tab and let the security audit type a remediation command into it. Every
//! question a caller may ask is a method here instead.

use serde::{Deserialize, Serialize};

use super::tabs::Tab;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum TabKind {
    #[default]
    Ssh,
    Mongodb,
    Redis,
    Vnc,
    /// A shell on this machine. It renders like SSH and takes commands, but
    /// has no host behind it, so no SFTP/Docker/Security.
    Local,
}

/// Which model action closes a tab.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CloseAction {
    /// Tear down a shell session (SSH or local).
    Disconnect,
    /// Close a datastore or VNC panel, releasing the host's connection when it
    /// was the last tab of that kind for that host.
    CloseServiceTab,
}

impl TabKind {
    /// A persisted type string. SSH tabs predate the field and carry none, and
    /// a type written by a future version must not break the tab strip, so
    /// anything unrecognised is SSH.
    pub fn from_type(ty: Option<&str>) -> Self {
        match ty {
            Some("mongodb") => Self::Mongodb,
            Some("redis") => Self::Redis,
            Some("vnc") => Self::Vnc,
            Some("local") => Self::Local,
            _ => Self::Ssh,
        }
    }

    /// Rendered by the terminal view: an SSH shell or a local one.
    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Ssh | Self::Local)
    }

    /// Only an SSH shell has SFTP / Docker / Security / Tunnels beside it.
    pub fn has_right_panel(self) -> bool {
        matches!(self, Self::Ssh)
    }

    /// Whether a panel may type a command into this tab. Connectedness is the
    /// caller's business; this answers only "is this the kind of tab that has
    /// a shell at all". A VNC tab forwards every keystroke to the remote
    /// desktop, so it must never be offered as somewhere to type either.
    pub fn accepts_commands(self) -> bool {
        matches!(self, Self::Ssh | Self::Local)
    }

    pub fn close_action(self) -> CloseAction {
        if self.is_terminal() {
            CloseAction::Disconnect
        } else {
            CloseAction::CloseServiceTab
        }
    }
}

/// `hasRightPanel(tab)`, with "no tab" answering false.
pub fn has_right_panel(tab: Option<&Tab>) -> bool {
    tab.is_some_and(|t| t.kind.has_right_panel())
}

/// `acceptsCommands(tab)`, with "no tab" answering false.
pub fn accepts_commands(tab: Option<&Tab>) -> bool {
    tab.is_some_and(|t| t.kind.accepts_commands())
}

pub fn close_action_for(tab: &Tab) -> CloseAction {
    tab.kind.close_action()
}

pub fn tabs_of_kind(tabs: &[Tab], kind: TabKind) -> Vec<&Tab> {
    tabs.iter().filter(|t| t.kind == kind).collect()
}

/// The title for a new local tab: the shell's label (`Local` for the default),
/// then `zsh 2`, `zsh 3`, taking the lowest number no open local tab is using.
/// An SSH tab that happens to share the name does not count.
pub fn next_local_tab_name(tabs: &[Tab], base: &str) -> String {
    let used: std::collections::HashSet<&str> = tabs
        .iter()
        .filter(|t| t.kind == TabKind::Local)
        .map(|t| t.title.as_str())
        .collect();
    if !used.contains(base) {
        return base.to_string();
    }
    let mut n = 2;
    while used.contains(format!("{base} {n}").as_str()) {
        n += 1;
    }
    format!("{base} {n}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tab(id: &str, kind: TabKind) -> Tab {
        Tab::new(id, None, kind, id)
    }
    fn named(kind: TabKind, title: &str) -> Tab {
        Tab::new(title, None, kind, title)
    }

    #[test]
    fn treats_a_tab_with_no_type_as_an_ssh_tab() {
        assert_eq!(TabKind::from_type(None), TabKind::Ssh);
    }

    #[test]
    fn reads_the_type_when_there_is_one() {
        assert_eq!(TabKind::from_type(Some("mongodb")), TabKind::Mongodb);
        assert_eq!(TabKind::from_type(Some("redis")), TabKind::Redis);
        assert_eq!(TabKind::from_type(Some("vnc")), TabKind::Vnc);
    }

    #[test]
    fn falls_back_to_ssh_for_an_unknown_type_rather_than_throwing() {
        assert_eq!(TabKind::from_type(Some("something-new")), TabKind::Ssh);
    }

    #[test]
    fn partitions_the_tab_list() {
        let tabs = vec![
            tab("1", TabKind::Ssh),
            tab("2", TabKind::Mongodb),
            tab("3", TabKind::Redis),
            tab("4", TabKind::Ssh),
        ];
        let ids = |k| {
            tabs_of_kind(&tabs, k)
                .iter()
                .map(|t| t.id.as_str())
                .collect::<Vec<_>>()
        };
        assert_eq!(ids(TabKind::Ssh), ["1", "4"]);
        assert_eq!(ids(TabKind::Mongodb), ["2"]);
        assert_eq!(ids(TabKind::Redis), ["3"]);
        assert!(tabs_of_kind(&[], TabKind::Ssh).is_empty());
    }

    #[test]
    fn has_right_panel_is_true_only_for_shell_tabs() {
        // With a `!= mongodb` check, a Redis tab would render the
        // SFTP/Docker/Security panel beside it.
        assert!(has_right_panel(Some(&tab("1", TabKind::Ssh))));
        assert!(!has_right_panel(Some(&tab("m", TabKind::Mongodb))));
        assert!(!has_right_panel(Some(&tab("r", TabKind::Redis))));
        assert!(!has_right_panel(Some(&tab("v", TabKind::Vnc))));
        assert!(!has_right_panel(None));
    }

    #[test]
    fn accepts_commands_is_true_only_for_shell_tabs() {
        // The security panel types remediation commands into a live root
        // shell; a datastore tab or a VNC tab must never be a target.
        assert!(accepts_commands(Some(&tab("1", TabKind::Ssh))));
        assert!(!accepts_commands(Some(&tab("m", TabKind::Mongodb))));
        assert!(!accepts_commands(Some(&tab("r", TabKind::Redis))));
        assert!(!accepts_commands(Some(&tab("v", TabKind::Vnc))));
        assert!(!accepts_commands(None));
    }

    #[test]
    fn close_action_routes_each_kind_to_its_model_action() {
        assert_eq!(
            close_action_for(&tab("1", TabKind::Ssh)),
            CloseAction::Disconnect
        );
        for k in [TabKind::Mongodb, TabKind::Redis, TabKind::Vnc] {
            assert_eq!(close_action_for(&tab("x", k)), CloseAction::CloseServiceTab);
        }
    }

    #[test]
    fn local_tabs_render_as_terminals_and_close_like_a_shell() {
        let local = tab("l", TabKind::Local);
        assert_eq!(TabKind::from_type(Some("local")), TabKind::Local);
        assert!(local.kind.is_terminal());
        assert!(TabKind::Ssh.is_terminal());
        assert!(!TabKind::Redis.is_terminal());
        assert_eq!(close_action_for(&local), CloseAction::Disconnect);
    }

    #[test]
    fn local_tabs_take_commands_but_get_no_right_panel() {
        // There is no host behind a local shell for those panels to talk to.
        let local = tab("l", TabKind::Local);
        assert!(!has_right_panel(Some(&local)));
        assert!(accepts_commands(Some(&local)));
        assert_ne!(local.kind, TabKind::Ssh);
    }

    #[test]
    fn local_tabs_are_named_local_local_2_reusing_the_lowest_free_number() {
        assert_eq!(next_local_tab_name(&[], "Local"), "Local");
        assert_eq!(
            next_local_tab_name(&[named(TabKind::Local, "Local")], "Local"),
            "Local 2"
        );
        assert_eq!(
            next_local_tab_name(
                &[
                    named(TabKind::Local, "Local"),
                    named(TabKind::Local, "Local 3")
                ],
                "Local"
            ),
            "Local 2"
        );
        // An SSH host that happens to be called Local does not count.
        assert_eq!(
            next_local_tab_name(&[named(TabKind::Ssh, "Local")], "Local"),
            "Local"
        );
    }

    #[test]
    fn local_tabs_are_named_after_the_chosen_shell() {
        let tabs = [
            named(TabKind::Local, "Ubuntu (WSL)"),
            named(TabKind::Local, "Local"),
        ];
        assert_eq!(next_local_tab_name(&tabs, "Ubuntu (WSL)"), "Ubuntu (WSL) 2");
        assert_eq!(next_local_tab_name(&tabs, "PowerShell"), "PowerShell");
    }
}
