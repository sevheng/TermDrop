//! What the sidebar's menus offer, without GPUI: the host, group and
//! empty-area context menus and the "Add a host" menu. Port of the context
//! menu templates in `HostSidebar.vue`, so which items appear for which row
//! is tested here rather than read off a screenshot.

use super::host_form::EditorKind;
use super::host_kind::HostKind;
use super::prefs::ViewMode;
use crate::backend::HostSummary;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HostMenuItem {
    /// "Connect" for an SSH host, "Open" for a panel host.
    Activate(&'static str),
    /// Only on an SSH host that also has a screen: its double-click stays a
    /// shell, so this is the way in.
    OpenVnc,
    Edit,
    /// Labelled by the current state: "Unfavorite" when it is one.
    ToggleFavorite {
        favorite: bool,
    },
    CopyName,
    Separator,
    Delete,
    /// The "Move to" heading, then one entry per group.
    MoveToLabel,
    MoveTo(String),
}

impl HostMenuItem {
    pub fn label(&self) -> String {
        match self {
            Self::Activate(verb) => (*verb).to_string(),
            Self::OpenVnc => "Open remote desktop".into(),
            Self::Edit => "Edit".into(),
            Self::ToggleFavorite { favorite: true } => "Unfavorite".into(),
            Self::ToggleFavorite { favorite: false } => "Favorite".into(),
            Self::CopyName => "Copy name".into(),
            Self::Separator => String::new(),
            Self::Delete => "Delete".into(),
            Self::MoveToLabel => "Move to".into(),
            Self::MoveTo(g) if g.is_empty() => "Ungrouped".into(),
            Self::MoveTo(g) => g.clone(),
        }
    }
}

/// The host row's menu. "Move to" appears in the grouped view only, where a
/// group is something you can see.
pub fn host_menu(host: &HostSummary, view: ViewMode, group_names: &[String]) -> Vec<HostMenuItem> {
    let mut items = vec![HostMenuItem::Activate(host.kind.activate_verb())];
    if host.has_vnc && host.kind == HostKind::Ssh {
        items.push(HostMenuItem::OpenVnc);
    }
    items.extend([
        HostMenuItem::Edit,
        HostMenuItem::ToggleFavorite {
            favorite: host.favorite,
        },
        HostMenuItem::CopyName,
        HostMenuItem::Separator,
        HostMenuItem::Delete,
    ]);
    if view == ViewMode::Grouped && !group_names.is_empty() {
        items.push(HostMenuItem::Separator);
        items.push(HostMenuItem::MoveToLabel);
        items.extend(group_names.iter().cloned().map(HostMenuItem::MoveTo));
    }
    items
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GroupMenuItem {
    /// Opens the SSH host dialog with this group preselected.
    AddHost,
    Rename,
    Delete,
}

impl GroupMenuItem {
    pub fn label(self) -> &'static str {
        match self {
            Self::AddHost => "Add Host",
            Self::Rename => "Rename",
            Self::Delete => "Delete group",
        }
    }
}

/// A group header's menu, with whether each item is enabled. Ungrouped is
/// not a group you can rename or delete; the Vue menu offered both and then
/// did nothing, so here they are shown disabled.
pub fn group_menu(name: &str) -> Vec<(GroupMenuItem, bool)> {
    let real = !name.is_empty();
    vec![
        (GroupMenuItem::AddHost, true),
        (GroupMenuItem::Rename, real),
        (GroupMenuItem::Delete, real),
    ]
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EmptyMenuItem {
    NewGroup,
    AddHost,
}

impl EmptyMenuItem {
    pub fn label(self) -> &'static str {
        match self {
            Self::NewGroup => "New Group",
            Self::AddHost => "Add Host",
        }
    }
}

/// The menu on the list's empty area. A new group only makes sense where
/// groups are shown.
pub fn empty_menu(view: ViewMode) -> Vec<EmptyMenuItem> {
    match view {
        ViewMode::Grouped => vec![EmptyMenuItem::NewGroup, EmptyMenuItem::AddHost],
        ViewMode::Flat => vec![EmptyMenuItem::AddHost],
    }
}

/// The header's "Add a host" menu: one entry per editor.
pub fn add_menu() -> [(EditorKind, &'static str); 4] {
    [
        (EditorKind::Ssh, "Host"),
        (EditorKind::Mongo, "MongoDB"),
        (EditorKind::Redis, "Redis"),
        (EditorKind::Vnc, "Remote desktop"),
    ]
}

/// The delete confirmation's text. A datastore row has no SSH host, and
/// "( )" would read as a bug.
pub fn delete_host_message(host: &HostSummary) -> String {
    if host.host.is_empty() {
        format!("Delete \"{}\"? This cannot be undone.", host.name)
    } else {
        format!(
            "Delete \"{}\" ({})? This cannot be undone.",
            host.name, host.host
        )
    }
}

pub fn delete_group_message(name: &str) -> String {
    format!("Delete group \"{name}\"? Hosts will be moved to Ungrouped.")
}

/// `GroupModal.vue`'s check. `existing` is every group name but the one
/// being renamed; `current` is that name, or `None` when creating.
pub fn group_name_error(
    name: &str,
    existing: &[String],
    current: Option<&str>,
) -> Option<&'static str> {
    let t = name.trim();
    if t.is_empty() {
        Some("Group name is required")
    } else if existing.iter().any(|g| g == t) {
        Some("A group with this name already exists")
    } else if current == Some(t) {
        Some("New name must be different")
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::HostRecord;

    fn host(f: impl FnOnce(&mut HostRecord)) -> HostSummary {
        let mut r = HostRecord {
            id: 1,
            name: "web".into(),
            host: "10.0.0.1".into(),
            ..Default::default()
        };
        f(&mut r);
        r.summary()
    }

    fn labels(items: &[HostMenuItem]) -> Vec<String> {
        items.iter().map(HostMenuItem::label).collect()
    }

    #[test]
    fn an_ssh_host_connects_and_can_move_between_groups() {
        let groups = vec![String::new(), "Prod".into()];
        let items = host_menu(&host(|_| {}), ViewMode::Grouped, &groups);
        assert_eq!(
            labels(&items),
            [
                "Connect",
                "Edit",
                "Favorite",
                "Copy name",
                "",
                "Delete",
                "",
                "Move to",
                "Ungrouped",
                "Prod"
            ]
        );
        // No group moves in the flat view.
        let flat = host_menu(&host(|_| {}), ViewMode::Flat, &groups);
        assert!(!flat.contains(&HostMenuItem::MoveToLabel));
        assert_eq!(flat.last(), Some(&HostMenuItem::Delete));
    }

    #[test]
    fn a_panel_host_opens_and_only_an_ssh_host_with_a_screen_offers_vnc() {
        let redis = host(|r| {
            r.host.clear();
            r.redis_uri = Some("redis://h".into());
        });
        let items = host_menu(&redis, ViewMode::Flat, &[]);
        assert_eq!(items[0], HostMenuItem::Activate("Open"));
        assert!(!items.contains(&HostMenuItem::OpenVnc));

        let vnc_only = host(|r| {
            r.host.clear();
            r.vnc_host = Some("10.0.0.5".into());
        });
        assert!(!host_menu(&vnc_only, ViewMode::Flat, &[]).contains(&HostMenuItem::OpenVnc));

        let ssh_screen = host(|r| r.vnc_host = Some("127.0.0.1".into()));
        assert_eq!(
            host_menu(&ssh_screen, ViewMode::Flat, &[])[1],
            HostMenuItem::OpenVnc
        );
    }

    #[test]
    fn the_favourite_item_names_what_it_will_do() {
        let fav = host(|r| r.favorite = true);
        assert!(labels(&host_menu(&fav, ViewMode::Flat, &[])).contains(&"Unfavorite".into()));
    }

    #[test]
    fn group_and_empty_menus() {
        assert_eq!(
            group_menu("Prod"),
            [
                (GroupMenuItem::AddHost, true),
                (GroupMenuItem::Rename, true),
                (GroupMenuItem::Delete, true)
            ]
        );
        assert!(!group_menu("")[1].1 && !group_menu("")[2].1);
        assert_eq!(
            empty_menu(ViewMode::Grouped),
            [EmptyMenuItem::NewGroup, EmptyMenuItem::AddHost]
        );
        assert_eq!(empty_menu(ViewMode::Flat), [EmptyMenuItem::AddHost]);
        assert_eq!(
            add_menu().map(|(_, l)| l),
            ["Host", "MongoDB", "Redis", "Remote desktop"]
        );
    }

    #[test]
    fn group_names_are_checked_as_group_modal_did() {
        let existing = vec!["Prod".to_string()];
        assert_eq!(
            group_name_error(" ", &existing, None),
            Some("Group name is required")
        );
        assert_eq!(
            group_name_error("Prod ", &existing, None),
            Some("A group with this name already exists")
        );
        assert_eq!(
            group_name_error("Lab", &existing, Some("Lab")),
            Some("New name must be different")
        );
        assert_eq!(group_name_error("Lab", &existing, None), None);
    }

    #[test]
    fn delete_messages_use_the_vue_wording() {
        assert_eq!(
            delete_host_message(&host(|_| {})),
            "Delete \"web\" (10.0.0.1)? This cannot be undone."
        );
        let mongo = host(|r| {
            r.host.clear();
            r.mongo_uri = Some("mongodb://h".into());
        });
        assert_eq!(
            delete_host_message(&mongo),
            "Delete \"web\"? This cannot be undone."
        );
        assert_eq!(
            delete_group_message("Prod"),
            "Delete group \"Prod\"? Hosts will be moved to Ungrouped."
        );
    }
}
