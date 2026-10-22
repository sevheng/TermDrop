//! The open tabs and which one is active, as plain data.
//!
//! Kept free of GPUI so the open/close/cycle rules are testable without a
//! window; `AppModel` owns one and notifies around every change.

use super::tab_kind::TabKind;

/// One open tab. For SSH tabs `id` will be the backend session id, as it is in
/// the Vue store, so a reconnect that reuses the session keeps the tab put.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tab {
    pub id: String,
    /// `None` for a local shell, which has no saved host behind it.
    pub host_id: Option<i64>,
    pub kind: TabKind,
    pub title: String,
    /// "There is a live session behind this tab." Datastore and VNC tabs are
    /// connected by virtue of existing, which is why `new` starts them true;
    /// a shell starts false and is flipped by its session's events.
    pub connected: bool,
    pub connecting: bool,
    /// For a local tab, the detected shell it was opened with (an id from
    /// `local_shells`, never a program path); `None` is the default shell.
    pub shell_id: Option<String>,
}

impl Tab {
    pub fn new(
        id: impl Into<String>,
        host_id: Option<i64>,
        kind: TabKind,
        title: impl Into<String>,
    ) -> Self {
        Self {
            id: id.into(),
            host_id,
            kind,
            title: title.into(),
            connected: !kind.is_terminal(),
            connecting: false,
            shell_id: None,
        }
    }
}

#[derive(Debug, Default, Clone)]
pub struct Tabs {
    tabs: Vec<Tab>,
    active: Option<String>,
}

impl Tabs {
    pub fn all(&self) -> &[Tab] {
        &self.tabs
    }

    pub fn active_id(&self) -> Option<&str> {
        self.active.as_deref()
    }

    pub fn active(&self) -> Option<&Tab> {
        let id = self.active.as_deref()?;
        self.get(id)
    }

    pub fn get(&self, id: &str) -> Option<&Tab> {
        self.tabs.iter().find(|t| t.id == id)
    }

    pub fn get_mut(&mut self, id: &str) -> Option<&mut Tab> {
        self.tabs.iter_mut().find(|t| t.id == id)
    }

    pub fn len(&self) -> usize {
        self.tabs.len()
    }

    pub fn is_empty(&self) -> bool {
        self.tabs.is_empty()
    }

    /// Append a tab and make it the active one, which is what opening a tab
    /// from anywhere in the app has always done.
    pub fn open(&mut self, tab: Tab) {
        self.active = Some(tab.id.clone());
        self.tabs.push(tab);
    }

    /// Make `id` active. False when no such tab exists, so a stale id from a
    /// late event cannot blank the tab area.
    pub fn activate(&mut self, id: &str) -> bool {
        if self.get(id).is_none() {
            return false;
        }
        self.active = Some(id.to_string());
        true
    }

    /// Remove a tab. Closing the active tab activates the one that slid into
    /// its place, or the one before it at the end of the strip, so the focus
    /// stays where the eye already is.
    pub fn close(&mut self, id: &str) -> Option<Tab> {
        let ix = self.tabs.iter().position(|t| t.id == id)?;
        let tab = self.tabs.remove(ix);
        if self.active.as_deref() == Some(id) {
            self.active = self
                .tabs
                .get(ix)
                .or_else(|| ix.checked_sub(1).and_then(|i| self.tabs.get(i)))
                .map(|t| t.id.clone());
        }
        Some(tab)
    }

    /// Ctrl+Tab / Ctrl+Shift+Tab. Wraps, as the Vue shortcut did: a tab strip
    /// is short enough that wrapping loses no one's place.
    pub fn cycle(&mut self, forward: bool) -> Option<&str> {
        let n = self.tabs.len();
        if n == 0 {
            return None;
        }
        let ix = self
            .active
            .as_deref()
            .and_then(|id| self.tabs.iter().position(|t| t.id == id));
        let next = match (ix, forward) {
            (None, _) => 0,
            (Some(i), true) => (i + 1) % n,
            (Some(i), false) => (i + n - 1) % n,
        };
        self.active = Some(self.tabs[next].id.clone());
        self.active.as_deref()
    }

    /// Whether `host_id` still has an open tab of `kind`. Kind as well as host,
    /// because a VNC tab can sit on an SSH host beside its shell, and without
    /// the kind the shell would suppress the VNC disconnect forever.
    pub fn host_has_tab_of_kind(&self, host_id: i64, kind: TabKind) -> bool {
        self.tabs
            .iter()
            .any(|t| t.host_id == Some(host_id) && t.kind == kind)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tabs(ids: &[&str]) -> Tabs {
        let mut t = Tabs::default();
        for id in ids {
            t.open(Tab::new(*id, Some(1), TabKind::Ssh, *id));
        }
        t
    }

    #[test]
    fn opening_activates_the_new_tab() {
        let t = tabs(&["a", "b"]);
        assert_eq!(t.active_id(), Some("b"));
        assert_eq!(t.len(), 2);
    }

    #[test]
    fn closing_the_active_tab_activates_its_neighbour() {
        let mut t = tabs(&["a", "b", "c"]);
        t.activate("b");
        t.close("b");
        assert_eq!(t.active_id(), Some("c"));
        t.close("c");
        assert_eq!(t.active_id(), Some("a"));
        t.close("a");
        assert_eq!(t.active_id(), None);
    }

    #[test]
    fn closing_another_tab_keeps_the_active_one() {
        let mut t = tabs(&["a", "b", "c"]);
        t.activate("a");
        t.close("c");
        assert_eq!(t.active_id(), Some("a"));
        assert!(t.close("missing").is_none());
    }

    #[test]
    fn activating_a_missing_tab_changes_nothing() {
        let mut t = tabs(&["a"]);
        assert!(!t.activate("zzz"));
        assert_eq!(t.active_id(), Some("a"));
    }

    #[test]
    fn cycling_wraps_both_ways() {
        let mut t = tabs(&["a", "b", "c"]);
        assert_eq!(t.cycle(true), Some("a"));
        assert_eq!(t.cycle(false), Some("c"));
        assert_eq!(t.cycle(false), Some("b"));
        assert_eq!(Tabs::default().cycle(true), None);
    }

    #[test]
    fn service_tabs_start_connected_and_shells_do_not() {
        assert!(Tab::new("r", Some(1), TabKind::Redis, "r").connected);
        assert!(!Tab::new("s", Some(1), TabKind::Ssh, "s").connected);
        assert!(!Tab::new("l", None, TabKind::Local, "l").connected);
    }

    #[test]
    fn last_tab_test_compares_kind_as_well_as_host() {
        let mut t = Tabs::default();
        t.open(Tab::new("s", Some(7), TabKind::Ssh, "s"));
        t.open(Tab::new("v", Some(7), TabKind::Vnc, "v"));
        t.close("v");
        assert!(!t.host_has_tab_of_kind(7, TabKind::Vnc));
        assert!(t.host_has_tab_of_kind(7, TabKind::Ssh));
    }
}
