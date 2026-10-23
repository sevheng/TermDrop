//! `AppModel`: the one entity that replaces the Pinia store
//! (`src/stores/connection.js`) for hosts, tabs and settings.
//!
//! Views read it and call its methods; nothing else mutates its state. Every
//! method that changes something notifies, and the changes another part of
//! the app must act on (a tab closing, the theme changing) are also emitted
//! as [`AppEvent`]s, which is where the integration step hangs session
//! teardown.

use std::collections::HashSet;
use std::sync::Arc;

use gpui_kit::{Context, EventEmitter};

use super::host_grouping::group_names;
use super::host_kind::HostKind;
use super::host_row_state::{HostRowState, host_row_state};
use super::prefs::{self, Prefs, ThemeChoice, ViewMode};
use super::tab_kind::{CloseAction, TabKind, next_local_tab_name};
use super::tabs::{SftpLink, Tab, Tabs};
use crate::backend::{
    HostRecord, HostStore, HostSummary, ImportEntry, ImportSummary, SecretKind, SshConfigEntry,
};
use crate::panels::PanelKind;

#[derive(Debug, Clone, PartialEq)]
pub enum AppEvent {
    TabOpened(String),
    /// A tab left the strip. `last_of_kind_for_host` is true when no other tab
    /// of the same kind is open for the same host: the moment a datastore's
    /// pooled connection, a VNC tunnel or an exec session may be released.
    TabClosed {
        tab: Tab,
        action: CloseAction,
        last_of_kind_for_host: bool,
    },
    ActiveTabChanged(Option<String>),
    HostsChanged,
    ThemeChanged(ThemeChoice),
    FontSizeChanged(u16),
    /// A setting written through [`AppModel::set_setting`] changed, by key.
    /// Panels that share a setting (every SFTP panel's `sftp-columns`)
    /// follow each other through this.
    SettingChanged(String),
    /// Something the user should hear about; the window shows it as a toast.
    Error(String),
}

pub struct AppModel {
    store: Arc<dyn HostStore>,
    hosts: Vec<HostSummary>,
    hosts_loaded: bool,
    tabs: Tabs,
    connecting_host_id: Option<i64>,
    prefs: Prefs,
    right_panel_open: bool,
    right_panel_tab: PanelKind,
    tab_seq: u64,
    local_shells: Vec<LocalShellChoice>,
}

/// A shell this machine has, as the new-terminal menu lists it. Only the id
/// goes back to the backend, which resolves it against its own detection, so
/// the UI can never name a program to run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalShellChoice {
    pub id: String,
    pub label: String,
}

impl EventEmitter<AppEvent> for AppModel {}

impl AppModel {
    /// Load hosts and preferences from `store`. Synchronous: both are a few
    /// rows of SQLite, and the window should open with the right widths and
    /// theme rather than flash defaults first.
    pub fn new(store: Arc<dyn HostStore>, _cx: &mut Context<Self>) -> Self {
        let prefs = Prefs::load(store.as_ref());
        let mut model = Self {
            store,
            hosts: Vec::new(),
            hosts_loaded: false,
            tabs: Tabs::default(),
            connecting_host_id: None,
            prefs,
            right_panel_open: true,
            right_panel_tab: PanelKind::default(),
            tab_seq: 0,
            local_shells: Vec::new(),
        };
        // A failed load leaves the list empty and `hosts_loaded` false; the
        // sidebar then shows its loading state rather than "no hosts yet".
        if let Ok(hosts) = model.store.hosts() {
            model.hosts = hosts;
            model.hosts_loaded = true;
        }
        model
    }

    pub fn store(&self) -> &Arc<dyn HostStore> {
        &self.store
    }

    // ---- hosts ---------------------------------------------------------

    pub fn hosts(&self) -> &[HostSummary] {
        &self.hosts
    }

    /// False until the first successful load, so an empty list can tell
    /// "still loading" from "nothing saved".
    pub fn hosts_loaded(&self) -> bool {
        self.hosts_loaded
    }

    pub fn host(&self, id: i64) -> Option<&HostSummary> {
        self.hosts.iter().find(|h| h.id == id)
    }

    pub fn reload_hosts(&mut self, cx: &mut Context<Self>) {
        match self.store.hosts() {
            Ok(hosts) => {
                self.hosts = hosts;
                self.hosts_loaded = true;
                cx.emit(AppEvent::HostsChanged);
            }
            Err(e) => cx.emit(AppEvent::Error(format!("Could not load hosts: {e}"))),
        }
        cx.notify();
    }

    pub fn set_favorite(&mut self, host_id: i64, favorite: bool, cx: &mut Context<Self>) {
        if let Err(e) = self.store.set_favorite(host_id, favorite) {
            cx.emit(AppEvent::Error(format!("Could not update the host: {e}")));
            return;
        }
        if let Some(h) = self.hosts.iter_mut().find(|h| h.id == host_id) {
            h.favorite = favorite;
        }
        cx.emit(AppEvent::HostsChanged);
        cx.notify();
    }

    /// Every column of one row, for an editor. Read fresh rather than kept,
    /// because the list holds display summaries only.
    pub fn host_record(&self, host_id: i64) -> Result<Option<HostRecord>, String> {
        self.store.host_record(host_id).map_err(|e| e.to_string())
    }

    /// Insert or rewrite a host, then reload the list. Returns its id.
    pub fn save_host(
        &mut self,
        id: Option<i64>,
        record: &HostRecord,
        cx: &mut Context<Self>,
    ) -> Result<i64, String> {
        let result = match id {
            Some(id) => self.store.update_host(id, record).map(|()| id),
            None => self.store.add_host(record),
        };
        self.reload_hosts(cx);
        result.map_err(|e| e.to_string())
    }

    /// Store a secret typed into an editor. Never called with an empty one.
    pub fn store_secret(&self, kind: SecretKind, host_id: i64, secret: &str) -> Result<(), String> {
        self.store
            .store_secret(kind, host_id, secret)
            .map_err(|e| e.to_string())
    }

    pub fn has_secret(&self, kind: SecretKind, host_id: i64) -> bool {
        self.store.has_secret(kind, host_id)
    }

    pub fn clear_secret(&self, kind: SecretKind, host_id: i64) -> Result<(), String> {
        self.store
            .clear_secret(kind, host_id)
            .map_err(|e| e.to_string())
    }

    /// Delete a host after the user confirmed. Every tab for it closes first,
    /// each the way its own kind closes, as the Vue sidebar's `deleteHost`
    /// did: the row's sessions must not outlive it.
    pub fn delete_host(&mut self, host_id: i64, cx: &mut Context<Self>) -> Result<(), String> {
        let open: Vec<String> = self
            .tabs
            .all()
            .iter()
            .filter(|t| t.host_id == Some(host_id))
            .map(|t| t.id.clone())
            .collect();
        for id in open {
            self.close_tab(&id, cx);
        }
        let result = self.store.delete_host(host_id);
        self.reload_hosts(cx);
        result.map_err(|e| e.to_string())
    }

    /// Move a host to `group` (empty for Ungrouped).
    pub fn set_host_group(
        &mut self,
        host_id: i64,
        group: &str,
        cx: &mut Context<Self>,
    ) -> Result<(), String> {
        let result = self.store.set_group(host_id, group);
        self.reload_hosts(cx);
        result.map_err(|e| e.to_string())
    }

    /// Rename a group: its hosts move, and the custom-group list and the
    /// collapsed state follow the new name.
    pub fn rename_group(
        &mut self,
        old: &str,
        new: &str,
        cx: &mut Context<Self>,
    ) -> Result<(), String> {
        let new = new.trim();
        if new == old {
            return Ok(());
        }
        let result = self.store.rename_group(old, new);
        self.reload_hosts(cx);
        result.map_err(|e| e.to_string())?;
        let custom = &mut self.prefs.custom_groups;
        custom.retain(|g| g != old);
        if !custom.iter().any(|g| g == new) {
            custom.push(new.to_string());
        }
        let raw = prefs::encode_list(&self.prefs.custom_groups);
        self.save(prefs::HOST_CUSTOM_GROUPS, &raw, cx);
        if self.is_group_collapsed(old) {
            self.prefs.collapsed_groups.retain(|g| g != old);
            self.prefs.collapsed_groups.push(new.to_string());
            let raw = prefs::encode_list(&self.prefs.collapsed_groups);
            self.save(prefs::HOST_COLLAPSED_GROUPS, &raw, cx);
        }
        cx.notify();
        Ok(())
    }

    /// Delete a group: its hosts become ungrouped and it leaves the
    /// custom-group list.
    pub fn delete_group(&mut self, name: &str, cx: &mut Context<Self>) -> Result<(), String> {
        let result = self.store.clear_group(name);
        self.reload_hosts(cx);
        result.map_err(|e| e.to_string())?;
        if self.prefs.custom_groups.iter().any(|g| g == name) {
            self.prefs.custom_groups.retain(|g| g != name);
            let raw = prefs::encode_list(&self.prefs.custom_groups);
            self.save(prefs::HOST_CUSTOM_GROUPS, &raw, cx);
        }
        cx.notify();
        Ok(())
    }

    /// Every group name in play: the hosts' and the user's empty ones.
    pub fn group_names(&self) -> Vec<String> {
        group_names(&self.hosts, &self.prefs.custom_groups)
    }

    pub fn export_hosts(&self) -> Result<String, String> {
        self.store.export_hosts().map_err(|e| e.to_string())
    }

    /// Import, then reload whatever happened: a partial import must still
    /// show up rather than look like nothing landed.
    pub fn import_hosts(
        &mut self,
        entries: Vec<ImportEntry>,
        cx: &mut Context<Self>,
    ) -> Result<ImportSummary, String> {
        let result = self.store.import_hosts(entries);
        self.reload_hosts(cx);
        result.map_err(|e| e.to_string())
    }

    pub fn parse_ssh_config(&self) -> Result<Vec<SshConfigEntry>, String> {
        self.store.parse_ssh_config().map_err(|e| e.to_string())
    }

    /// Point a host at its jump host, rewriting only that column.
    pub fn set_jump_host(&mut self, host_id: i64, jump_id: i64) -> Result<(), String> {
        let mut rec = self
            .store
            .host_record(host_id)
            .map_err(|e| e.to_string())?
            .ok_or_else(|| format!("host {host_id} not found"))?;
        rec.jump_host_id = Some(jump_id);
        self.store
            .update_host(host_id, &rec)
            .map_err(|e| e.to_string())
    }

    pub fn row_state(&self, host_id: i64) -> HostRowState {
        host_row_state(
            self.tabs.all(),
            self.tabs.active_id(),
            self.connecting_host_id,
            host_id,
        )
    }

    pub fn connecting_host_id(&self) -> Option<i64> {
        self.connecting_host_id
    }

    pub fn set_connecting_host(&mut self, host_id: Option<i64>, cx: &mut Context<Self>) {
        self.connecting_host_id = host_id;
        cx.notify();
    }

    // ---- tabs ----------------------------------------------------------

    pub fn tabs(&self) -> &[Tab] {
        self.tabs.all()
    }

    pub fn tab(&self, id: &str) -> Option<&Tab> {
        self.tabs.get(id)
    }

    pub fn active_tab(&self) -> Option<&Tab> {
        self.tabs.active()
    }

    pub fn active_tab_id(&self) -> Option<&str> {
        self.tabs.active_id()
    }

    fn next_id(&mut self, prefix: &str) -> String {
        self.tab_seq += 1;
        format!("{prefix}-{}", self.tab_seq)
    }

    /// What double-clicking a host row does: a shell for an SSH host, the
    /// panel tab for a datastore or VNC-only host. Returns the tab's id.
    pub fn activate_host(&mut self, host_id: i64, cx: &mut Context<Self>) -> Option<String> {
        let kind = self.host(host_id)?.kind;
        match kind {
            HostKind::Ssh => self.open_ssh(host_id, cx),
            HostKind::Mongodb => self.open_service(host_id, TabKind::Mongodb, cx),
            HostKind::Redis => self.open_service(host_id, TabKind::Redis, cx),
            HostKind::Vnc => self.open_service(host_id, TabKind::Vnc, cx),
            HostKind::Local => self.open_local(None, cx),
        }
    }

    /// A new shell tab for an SSH host. Every call opens another one, as the
    /// Vue app does: two shells on one host is ordinary. The tab starts
    /// `connecting`; the session code flips it with [`Self::set_tab_status`].
    pub fn open_ssh(&mut self, host_id: i64, cx: &mut Context<Self>) -> Option<String> {
        let host = self.host(host_id)?;
        let title = host.name.clone();
        let id = self.next_id("ssh");
        let mut tab = Tab::new(id.clone(), Some(host_id), TabKind::Ssh, title);
        tab.connecting = true;
        self.push_tab(tab, cx);
        Some(id)
    }

    /// The remote-desktop tab of a host that has a screen, SSH or VNC-only.
    pub fn open_vnc(&mut self, host_id: i64, cx: &mut Context<Self>) -> Option<String> {
        self.host(host_id).filter(|h| h.has_vnc)?;
        self.open_service(host_id, TabKind::Vnc, cx)
    }

    /// Datastore and VNC tabs are one per (host, kind): opening again
    /// activates the one already there.
    fn open_service(
        &mut self,
        host_id: i64,
        kind: TabKind,
        cx: &mut Context<Self>,
    ) -> Option<String> {
        if let Some(existing) = self
            .tabs
            .all()
            .iter()
            .find(|t| t.host_id == Some(host_id) && t.kind == kind)
            .map(|t| t.id.clone())
        {
            self.activate_tab(&existing, cx);
            return Some(existing);
        }
        let title = self.host(host_id)?.name.clone();
        let id = self.next_id(kind_prefix(kind));
        self.push_tab(Tab::new(id.clone(), Some(host_id), kind, title), cx);
        Some(id)
    }

    /// A shell on this machine. `shell_label` names the tab after the chosen
    /// shell (`zsh`, `Ubuntu (WSL)`); `None` is the default shell, `Local`.
    pub fn open_local(
        &mut self,
        shell_label: Option<&str>,
        cx: &mut Context<Self>,
    ) -> Option<String> {
        let title = next_local_tab_name(self.tabs.all(), shell_label.unwrap_or("Local"));
        let id = self.next_id("local");
        let mut tab = Tab::new(id.clone(), None, TabKind::Local, title);
        tab.connecting = true;
        self.push_tab(tab, cx);
        Some(id)
    }

    /// A local tab for one detected shell, titled after it (`zsh`,
    /// `Ubuntu (WSL)`).
    pub fn open_local_shell(
        &mut self,
        shell: &LocalShellChoice,
        cx: &mut Context<Self>,
    ) -> Option<String> {
        let title = next_local_tab_name(self.tabs.all(), &shell.label);
        let id = self.next_id("local");
        let mut tab = Tab::new(id.clone(), None, TabKind::Local, title);
        tab.connecting = true;
        tab.shell_id = Some(shell.id.clone());
        self.push_tab(tab, cx);
        Some(id)
    }

    /// The shells the new-terminal button offers. With more than one it
    /// becomes a menu; with one or none it opens the default directly.
    pub fn local_shells(&self) -> &[LocalShellChoice] {
        &self.local_shells
    }

    pub fn set_local_shells(&mut self, shells: Vec<LocalShellChoice>, cx: &mut Context<Self>) {
        self.local_shells = shells;
        cx.notify();
    }

    fn push_tab(&mut self, tab: Tab, cx: &mut Context<Self>) {
        let id = tab.id.clone();
        self.tabs.open(tab);
        cx.emit(AppEvent::TabOpened(id.clone()));
        cx.emit(AppEvent::ActiveTabChanged(Some(id)));
        cx.notify();
    }

    pub fn activate_tab(&mut self, id: &str, cx: &mut Context<Self>) {
        if self.tabs.active_id() == Some(id) {
            return;
        }
        if self.tabs.activate(id) {
            cx.emit(AppEvent::ActiveTabChanged(Some(id.to_string())));
            cx.notify();
        }
    }

    pub fn cycle_tabs(&mut self, forward: bool, cx: &mut Context<Self>) {
        let before = self.tabs.active_id().map(str::to_string);
        let after = self.tabs.cycle(forward).map(str::to_string);
        if before != after {
            cx.emit(AppEvent::ActiveTabChanged(after));
            cx.notify();
        }
    }

    /// Remove a tab. The caller has already confirmed with the user; this is
    /// the teardown, routed by kind through [`CloseAction`].
    pub fn close_tab(&mut self, id: &str, cx: &mut Context<Self>) -> Option<Tab> {
        let before = self.tabs.active_id().map(str::to_string);
        let tab = self.tabs.close(id)?;
        let last_of_kind_for_host = tab
            .host_id
            .is_none_or(|h| !self.tabs.host_has_tab_of_kind(h, tab.kind));
        cx.emit(AppEvent::TabClosed {
            action: tab.kind.close_action(),
            tab: tab.clone(),
            last_of_kind_for_host,
        });
        let after = self.tabs.active_id().map(str::to_string);
        if before != after {
            cx.emit(AppEvent::ActiveTabChanged(after));
        }
        cx.notify();
        Some(tab)
    }

    /// Session events land here: `ssh-connected`, `ssh-disconnected` and the
    /// rest in the Tauri app.
    pub fn set_tab_status(
        &mut self,
        id: &str,
        connected: bool,
        connecting: bool,
        cx: &mut Context<Self>,
    ) {
        if let Some(tab) = self.tabs.get_mut(id) {
            tab.connected = connected;
            tab.connecting = connecting;
            cx.notify();
        }
    }

    /// The tab's SFTP connection opened, failed, or is being retried.
    pub fn set_tab_sftp(&mut self, id: &str, sftp: SftpLink, cx: &mut Context<Self>) {
        if let Some(tab) = self.tabs.get_mut(id)
            && tab.sftp != sftp
        {
            tab.sftp = sftp;
            cx.notify();
        }
    }

    // ---- layout preferences -------------------------------------------

    pub fn prefs(&self) -> &Prefs {
        &self.prefs
    }

    fn save(&self, key: &str, value: &str, cx: &mut Context<Self>) {
        if let Err(e) = self.store.set_setting(key, value) {
            cx.emit(AppEvent::Error(format!("Could not save {key}: {e}")));
        }
    }

    /// Set while dragging; `persist` once the drag ends, because a width
    /// changes on every pointer move and the store needs only the last one.
    pub fn set_sidebar_width(&mut self, width: f32, persist: bool, cx: &mut Context<Self>) {
        self.prefs.sidebar_width = prefs::clamp_width(width, prefs::SIDEBAR_RANGE);
        if persist {
            let raw = (self.prefs.sidebar_width.round() as i64).to_string();
            self.save(prefs::SIDEBAR_WIDTH, &raw, cx);
        }
        cx.notify();
    }

    pub fn set_sftp_width(&mut self, width: f32, persist: bool, cx: &mut Context<Self>) {
        self.prefs.sftp_width = prefs::clamp_width(width, prefs::SFTP_RANGE);
        if persist {
            let raw = (self.prefs.sftp_width.round() as i64).to_string();
            self.save(prefs::SFTP_WIDTH, &raw, cx);
        }
        cx.notify();
    }

    /// The right panel only exists beside a tab that has one; this is whether
    /// the user has it open there. Per run, like the Vue app's panel state.
    pub fn right_panel_open(&self) -> bool {
        self.right_panel_open
    }

    pub fn toggle_right_panel(&mut self, cx: &mut Context<Self>) {
        self.right_panel_open = !self.right_panel_open;
        cx.notify();
    }

    /// Which of the right panel's tabs is selected. One choice for every SSH
    /// tab, in memory, starting at SFTP: `rightPanelTab` in `MainWindow.vue`.
    pub fn right_panel_tab(&self) -> PanelKind {
        self.right_panel_tab
    }

    pub fn set_right_panel_tab(&mut self, kind: PanelKind, cx: &mut Context<Self>) {
        if self.right_panel_tab != kind {
            self.right_panel_tab = kind;
            cx.notify();
        }
    }

    /// Whether panel `kind` for `host_id` is on screen: the panel is open,
    /// `kind` is selected, and the active tab is an SSH tab on that host
    /// (and, for a per-tab panel, is `tab_id`). A panel that polls (Docker)
    /// stops while this is false, as the Vue panels did on deactivation.
    pub fn panel_shown(&self, kind: PanelKind, host_id: i64, tab_id: Option<&str>) -> bool {
        crate::panels::panel_visible(
            self.right_panel_open,
            self.right_panel_tab,
            self.active_tab(),
            kind,
            host_id,
            tab_id,
        )
    }

    pub fn view_mode(&self) -> ViewMode {
        self.prefs.view_mode
    }

    pub fn toggle_view_mode(&mut self, cx: &mut Context<Self>) {
        self.prefs.view_mode = match self.prefs.view_mode {
            ViewMode::Grouped => ViewMode::Flat,
            ViewMode::Flat => ViewMode::Grouped,
        };
        self.save(prefs::HOST_VIEW_MODE, self.prefs.view_mode.as_str(), cx);
        cx.notify();
    }

    pub fn collapsed_groups(&self) -> HashSet<String> {
        self.prefs.collapsed_groups.iter().cloned().collect()
    }

    pub fn is_group_collapsed(&self, name: &str) -> bool {
        self.prefs.collapsed_groups.iter().any(|g| g == name)
    }

    pub fn set_group_collapsed(&mut self, name: &str, collapsed: bool, cx: &mut Context<Self>) {
        let list = &mut self.prefs.collapsed_groups;
        let present = list.iter().any(|g| g == name);
        match (collapsed, present) {
            (true, false) => list.push(name.to_string()),
            (false, true) => list.retain(|g| g != name),
            _ => return,
        }
        let raw = prefs::encode_list(&self.prefs.collapsed_groups);
        self.save(prefs::HOST_COLLAPSED_GROUPS, &raw, cx);
        cx.notify();
    }

    pub fn toggle_group(&mut self, name: &str, cx: &mut Context<Self>) {
        let collapsed = self.is_group_collapsed(name);
        self.set_group_collapsed(name, !collapsed, cx);
    }

    pub fn custom_groups(&self) -> &[String] {
        &self.prefs.custom_groups
    }

    /// A group the user created before putting any host in it. It starts
    /// expanded, so it does not inherit a collapsed state from an old group
    /// of the same name.
    pub fn add_custom_group(&mut self, name: &str, cx: &mut Context<Self>) {
        let name = name.trim();
        if name.is_empty() || self.prefs.custom_groups.iter().any(|g| g == name) {
            return;
        }
        self.prefs.custom_groups.push(name.to_string());
        let raw = prefs::encode_list(&self.prefs.custom_groups);
        self.save(prefs::HOST_CUSTOM_GROUPS, &raw, cx);
        self.set_group_collapsed(name, false, cx);
        cx.notify();
    }

    // ---- settings ------------------------------------------------------

    /// A raw setting from the store (the `settings` table), for a panel's
    /// own preference. `None` when unset or unreadable.
    pub fn setting(&self, key: &str) -> Option<String> {
        match self.store.get_setting(key) {
            Ok(value) => value,
            Err(e) => {
                tracing::warn!("Could not read {key}: {e}");
                None
            }
        }
    }

    /// Save a raw setting and tell every subscriber
    /// ([`AppEvent::SettingChanged`]), so other views showing it follow.
    /// Nothing is sent when the value is unchanged.
    pub fn set_setting(&mut self, key: &str, value: &str, cx: &mut Context<Self>) {
        if self.setting(key).as_deref() == Some(value) {
            return;
        }
        if let Err(e) = self.store.set_setting(key, value) {
            cx.emit(AppEvent::Error(format!("Could not save {key}: {e}")));
            return;
        }
        cx.emit(AppEvent::SettingChanged(key.to_string()));
    }

    pub fn theme(&self) -> ThemeChoice {
        self.prefs.theme
    }

    pub fn set_theme(&mut self, theme: ThemeChoice, cx: &mut Context<Self>) {
        self.prefs.theme = theme;
        self.save(prefs::THEME, theme.as_str(), cx);
        cx.emit(AppEvent::ThemeChanged(theme));
        cx.notify();
    }

    pub fn font_size(&self) -> u16 {
        self.prefs.font_size
    }

    pub fn set_font_size(&mut self, size: u16, cx: &mut Context<Self>) {
        let size = size.clamp(prefs::FONT_SIZE_RANGE.0, prefs::FONT_SIZE_RANGE.1);
        self.prefs.font_size = size;
        self.save(prefs::FONT_SIZE, &size.to_string(), cx);
        cx.emit(AppEvent::FontSizeChanged(size));
        cx.notify();
    }
}

fn kind_prefix(kind: TabKind) -> &'static str {
    match kind {
        TabKind::Ssh => "ssh",
        TabKind::Mongodb => "mongo",
        TabKind::Redis => "redis",
        TabKind::Vnc => "vnc",
        TabKind::Local => "local",
    }
}
