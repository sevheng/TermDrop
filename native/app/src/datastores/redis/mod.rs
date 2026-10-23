//! The Redis tab: port of `RedisPanel.vue`, `RedisKeyList.vue`,
//! `RedisKeyTree.vue`, `RedisValueView.vue`, `RedisRestoreDialog.vue` and
//! their composables (`useRedisConnection`, `useRedisKeys`,
//! `useRedisBackup`). Built against the contract in `../README.md`.
//!
//! The tab owns its connection, as the Vue panel did, so it appears at once
//! and shows its own spinner and error banner: for a tunnelled host the
//! banner is the only clue about the bastion.
//!
//! The rules from `CLAUDE.md` hold here by construction: every read is one
//! of `redis_scan_keys`, `redis_key_tree` or `redis_key_value`, a page at a
//! time, and a page is never trimmed; keys are addressed only by `key_b64`;
//! each call names its database and none issues `SELECT`; the tunnel and the
//! pooled connections belong to the core, and the frame releases them when
//! the host's last Redis tab closes.

mod logic;
mod restore;

use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::select::{Select, SelectEvent, SelectState};
use gpui_kit::component::spinner::Spinner;
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{Disableable as _, Icon, IndexPath, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, AnyView, App, AppContext as _, ClipboardItem, Context, Entity,
    InteractiveElement as _, IntoElement, ParentElement as _, Render, SharedString,
    StatefulInteractiveElement as _, Styled as _, Subscription, Task, Window, div, px,
    uniform_list,
};
use termdrop_core::redis::{
    KeyTree, MAX_DUMP_KEY_BYTES, MAX_ELEMENTS_PER_PAGE, MAX_KEYS_PER_PAGE, RedisKeySummary,
    RedisOpSummary, RedisServerInfo, RedisValuePage,
};

use self::logic::{
    CursorStack, EmptyAction, KEY_TYPES, MoreValue, TreeRow, append_page, clip_cell,
    copyable_value, databases_or_default, db_key_count, default_backup_name, default_restore_db,
    display_lines, empty_page, field_header, format_key_kind, format_ttl, group_label,
    is_viewable_kind, key_label, more_value, op_error_message, prefix_pattern, restore_warning,
    summarize_op, summarize_scan, total_label, tree_rows, truncated_note,
};
use self::restore::{RestoreChoice, open_restore_dialog};
use super::common::{OpProgress, format_bytes, format_count, op_progress_row, text_2xs};
use super::{OpSubscription, ServiceTabContext};
use crate::app::AppEvent;
use crate::panels::common::{list_head, list_row};
use crate::ui::choice::Choice;
use crate::ui::empty_state::EmptyState;
use crate::ui::icon_button::IconButton;
use crate::ui::panel_header::PanelHeader;
use crate::ui::tag::{Tag, TagTone};
use crate::ui::theme::{Palette, mono_font, palette};
use crate::ui::toast::ToastKind;

const ROW_H: f32 = 24.;
const TREE_W: f32 = 256.;
const VALUE_W: f32 = 384.;
const TTL_W: f32 = 112.;
const TYPE_W: f32 = 96.;
const SIZE_W: f32 = 96.;
const BACKUP_FILTER: (&str, &[&str]) = ("TermDrop Redis backup", &["tdredis"]);

/// The tab's view, built once when the tab opens and dropped when it closes.
pub fn build(ctx: ServiceTabContext, window: &mut Window, cx: &mut App) -> AnyView {
    cx.new(|cx| RedisTab::new(ctx, window, cx)).into()
}

type TypeSelect = Entity<SelectState<Vec<Choice<String>>>>;

struct RedisTab {
    ctx: ServiceTabContext,
    /// The stored URI with its credentials masked; re-read when hosts change.
    display_uri: String,
    /// The SSH host the tunnel goes through, by name ("SSH" when unknown).
    tunnel_name: String,

    // ---- the connection (`useRedisConnection`) ----
    info: Option<RedisServerInfo>,
    connecting: bool,
    connect_error: Option<String>,
    refreshing: bool,
    connect_task: Option<Task<()>>,

    // ---- the database and its tree ----
    active_db: i64,
    tree: KeyTree,
    tree_loading: bool,
    tree_seq: u64,
    tree_task: Option<Task<()>>,
    /// The group whose pattern is in the box, while it still is.
    active_group: Option<String>,

    // ---- the key list (`useRedisKeys`) ----
    pattern_input: Entity<InputState>,
    pattern: String,
    type_select: TypeSelect,
    type_filter: String,
    with_memory: bool,
    keys: Vec<RedisKeySummary>,
    keys_loading: bool,
    keys_error: Option<String>,
    keys_done: bool,
    stack: CursorStack,
    /// Where the next page starts, held until it is asked for.
    pending: Option<String>,
    page_number: usize,
    can_go_back: bool,
    keys_seq: u64,
    keys_task: Option<Task<()>>,

    // ---- the value ----
    active_key: Option<RedisKeySummary>,
    value: Option<RedisValuePage>,
    /// A string value's text, split as it is drawn.
    value_lines: Vec<SharedString>,
    value_loading: bool,
    /// The load is a "Load more": keep the shown page on screen.
    value_appending: bool,
    value_error: Option<String>,
    value_seq: u64,
    value_task: Option<Task<()>>,

    // ---- backup and restore (`useRedisBackup`) ----
    op: Option<OpProgress>,
    op_sub: Option<OpSubscription>,
    op_task: Option<Task<()>>,
    header_task: Option<Task<()>>,

    _subscriptions: Vec<Subscription>,
}

impl RedisTab {
    fn new(ctx: ServiceTabContext, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let pattern_input =
            cx.new(|cx| InputState::new(window, cx).placeholder("Match pattern, e.g. user:*"));
        let choices: Vec<Choice<String>> = std::iter::once(Choice::new(String::new(), "All types"))
            .chain(
                KEY_TYPES
                    .iter()
                    .map(|t| Choice::new(t.to_string(), format_key_kind(t))),
            )
            .collect();
        let type_select =
            cx.new(|cx| SelectState::new(choices, Some(IndexPath::new(0)), window, cx));

        let subscriptions = vec![
            cx.subscribe_in(
                &pattern_input,
                window,
                |this, input, event: &InputEvent, window, cx| match event {
                    InputEvent::Change => this.pattern = input.read(cx).value().to_string(),
                    InputEvent::PressEnter { .. } => this.on_search(window, cx),
                    _ => {}
                },
            ),
            // `SelectMenu` emitted `change` only on a real change.
            cx.subscribe_in(
                &type_select,
                window,
                |this, _, event: &SelectEvent<Vec<Choice<String>>>, window, cx| {
                    let SelectEvent::Confirm(choice) = event;
                    let choice = choice.clone().unwrap_or_default();
                    if choice != this.type_filter {
                        this.type_filter = choice;
                        this.on_search(window, cx);
                    }
                },
            ),
            cx.subscribe(&ctx.model, |this, _, event: &AppEvent, cx| {
                if matches!(event, AppEvent::HostsChanged) {
                    this.read_host(cx);
                    cx.notify();
                }
            }),
            // Nothing is left to show a running backup's result once the tab
            // is gone, so it stops with the tab. (The Vue toast outlived the
            // panel; here the result would arrive at nothing.)
            cx.on_release(|this, _| {
                if let Some(op) = this.op.as_ref().filter(|op| op.is_active()) {
                    this.ctx.cancel_op(&op.op_id);
                }
            }),
        ];

        let mut this = Self {
            ctx,
            display_uri: String::new(),
            tunnel_name: "SSH".into(),
            info: None,
            connecting: false,
            connect_error: None,
            refreshing: false,
            connect_task: None,
            active_db: 0,
            tree: empty_tree(),
            tree_loading: false,
            tree_seq: 0,
            tree_task: None,
            active_group: None,
            pattern_input,
            pattern: String::new(),
            type_select,
            type_filter: String::new(),
            with_memory: false,
            keys: Vec::new(),
            keys_loading: false,
            keys_error: None,
            keys_done: false,
            stack: CursorStack::default(),
            pending: None,
            page_number: 1,
            can_go_back: false,
            keys_seq: 0,
            keys_task: None,
            active_key: None,
            value: None,
            value_lines: Vec::new(),
            value_loading: false,
            value_appending: false,
            value_error: None,
            value_seq: 0,
            value_task: None,
            op: None,
            op_sub: None,
            op_task: None,
            header_task: None,
            _subscriptions: subscriptions,
        };
        this.read_host(cx);
        this.connect(window, cx);
        this
    }

    /// The masked address and the tunnel's name, from the host's row.
    fn read_host(&mut self, cx: &mut Context<Self>) {
        self.display_uri = self.ctx.display_uri(cx);
        let via = self
            .ctx
            .host_record(cx)
            .ok()
            .flatten()
            .and_then(|r| r.redis_tunnel_host_id);
        self.tunnel_name = via
            .and_then(|id| self.ctx.model.read(cx).host(id).map(|h| h.name.clone()))
            .unwrap_or_else(|| "SSH".into());
    }

    fn busy(&self) -> bool {
        self.op.as_ref().is_some_and(OpProgress::is_active)
    }

    fn is_cluster(&self) -> bool {
        self.info.as_ref().is_some_and(|i| i.mode == "cluster")
    }

    fn databases(&self) -> Vec<termdrop_core::redis::RedisDbInfo> {
        databases_or_default(self.info.as_ref().map_or(&[][..], |i| &i.databases))
    }

    // ---- the connection ------------------------------------------------

    /// `connect()`: with the password prompt-and-retry, then the first page
    /// and the tree of the first database.
    fn connect(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.connecting = true;
        self.connect_error = None;
        cx.notify();
        let host_id = self.ctx.host_id;
        self.connect_task =
            Some(
                self.ctx.run_with_secret(
                    window,
                    cx,
                    move |core, ssh_password| async move {
                        core.redis_connect(host_id, ssh_password).await
                    },
                    |this, result, window, cx| this.connected(result, window, cx),
                ),
            );
    }

    fn connected(
        &mut self,
        result: Result<RedisServerInfo, String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.connecting = false;
        match result {
            Ok(info) => {
                self.active_db = databases_or_default(&info.databases)[0].index;
                self.info = Some(info);
                self.read_host(cx);
                self.restart_keys(window, cx);
                self.load_tree(window, cx);
            }
            // A banner with Reconnect rather than a toast: it is the state
            // of the tab, not a passing event.
            Err(e) => {
                self.connect_error = Some(e);
                self.info = None;
            }
        }
        cx.notify();
    }

    /// `refreshAll`: re-read INFO so the key counts follow a backup or
    /// restore, then the page and the tree again. Without a connection it
    /// connects.
    fn refresh_all(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.info.is_none() {
            self.connect(window, cx);
            return;
        }
        self.refreshing = true;
        cx.notify();
        let core = self.ctx.core().clone();
        let host_id = self.ctx.host_id;
        self.connect_task = Some(self.ctx.run(
            window,
            cx,
            async move { core.redis_server_info(host_id).await },
            |this, result, window, cx| {
                this.refreshing = false;
                match result {
                    Ok(info) => this.info = Some(info),
                    Err(e) => this.ctx.toast(
                        window,
                        cx,
                        format!("Could not refresh the server info: {e}"),
                        ToastKind::Error,
                    ),
                }
                this.restart_keys(window, cx);
                this.load_tree(window, cx);
                cx.notify();
            },
        ));
    }

    // ---- the tree ------------------------------------------------------

    fn load_tree(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.tree_loading = true;
        self.tree_seq += 1;
        let seq = self.tree_seq;
        let core = self.ctx.core().clone();
        let (host_id, db) = (self.ctx.host_id, self.active_db);
        self.tree_task = Some(self.ctx.run(
            window,
            cx,
            async move { core.redis_key_tree(host_id, db, None).await },
            move |this, result, window, cx| {
                if seq != this.tree_seq {
                    return;
                }
                this.tree_loading = false;
                match result {
                    Ok(tree) => this.tree = tree,
                    Err(e) => {
                        this.ctx.toast(
                            window,
                            cx,
                            format!("Could not group the keys: {e}"),
                            ToastKind::Error,
                        );
                        this.tree = empty_tree();
                    }
                }
                cx.notify();
            },
        ));
        cx.notify();
    }

    fn select_db(&mut self, index: i64, window: &mut Window, cx: &mut Context<Self>) {
        if index == self.active_db {
            return;
        }
        self.active_db = index;
        self.active_group = None;
        self.clear_value();
        self.set_pattern("", window, cx);
        self.restart_keys(window, cx);
        self.load_tree(window, cx);
    }

    /// Narrow the server-side scan, not just the list drawn: clicking a group
    /// is only useful if it changes what Redis iterates.
    fn select_group(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(group) = self.tree.groups.get(ix).cloned() else {
            return;
        };
        if group.binary {
            self.ctx.toast(
                window,
                cx,
                "That prefix is not text, so it cannot be used as a match pattern",
                ToastKind::Warning,
            );
            return;
        }
        let pattern = prefix_pattern(&group.prefix);
        self.active_group = Some(group.prefix);
        self.set_pattern(&pattern, window, cx);
        self.restart_keys(window, cx);
    }

    fn set_pattern(&mut self, pattern: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.pattern = pattern.to_string();
        let value = pattern.to_string();
        self.pattern_input
            .update(cx, |s, cx| s.set_value(value, window, cx));
    }

    // ---- the key list --------------------------------------------------

    fn on_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.active_group = None;
        self.restart_keys(window, cx);
    }

    /// Start the iteration over, after a pattern, filter or database change.
    fn restart_keys(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.stack = CursorStack::default();
        self.pending = None;
        self.keys_done = false;
        self.load_keys("0".into(), window, cx);
    }

    fn next_page(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(cursor) = self.pending.clone() else {
            return;
        };
        self.stack.push(cursor.clone());
        self.load_keys(cursor, window, cx);
    }

    fn prev_page(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.stack.can_go_back() {
            return;
        }
        let cursor = self.stack.back();
        self.load_keys(cursor, window, cx);
    }

    fn load_keys(&mut self, cursor: String, window: &mut Window, cx: &mut Context<Self>) {
        self.keys_loading = true;
        // Inline, not a toast: a bad pattern is corrected in place.
        self.keys_error = None;
        self.keys_seq += 1;
        let seq = self.keys_seq;
        let core = self.ctx.core().clone();
        let (host_id, db) = (self.ctx.host_id, self.active_db);
        let pattern = Some(self.pattern.clone()).filter(|p| !p.is_empty());
        let type_filter = Some(self.type_filter.clone()).filter(|t| !t.is_empty());
        let with_memory = self.with_memory;
        let supports_scan_type = self.info.as_ref().is_some_and(|i| i.supports_scan_type);
        self.keys_task = Some(self.ctx.run(
            window,
            cx,
            async move {
                core.redis_scan_keys(
                    host_id,
                    db,
                    cursor,
                    pattern,
                    type_filter,
                    MAX_KEYS_PER_PAGE,
                    with_memory,
                    supports_scan_type,
                )
                .await
            },
            move |this, result, _, cx| {
                if seq != this.keys_seq {
                    return;
                }
                this.keys_loading = false;
                match result {
                    // The whole page, never trimmed: the cursor is only valid
                    // for the batch it came with.
                    Ok(page) => {
                        this.keys = page.keys;
                        this.keys_done = page.done;
                        this.pending = (!page.done).then_some(page.cursor);
                    }
                    Err(e) => {
                        this.keys_error = Some(e);
                        this.keys.clear();
                        this.pending = None;
                    }
                }
                this.page_number = this.stack.page_number();
                this.can_go_back = this.stack.can_go_back();
                cx.notify();
            },
        ));
        cx.notify();
    }

    fn set_with_memory(&mut self, on: bool, window: &mut Window, cx: &mut Context<Self>) {
        self.with_memory = on;
        self.on_search(window, cx);
    }

    fn clear_filter(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.set_pattern("", window, cx);
        self.type_filter.clear();
        self.type_select.update(cx, |s, cx| {
            s.set_selected_index(Some(IndexPath::new(0)), window, cx)
        });
        self.on_search(window, cx);
    }

    // ---- the value -----------------------------------------------------

    fn clear_value(&mut self) {
        self.active_key = None;
        self.value = None;
        self.value_lines.clear();
        self.value_error = None;
        self.value_loading = false;
        // Drop the answer of any read still in flight.
        self.value_seq += 1;
        self.value_task = None;
    }

    fn open_key(&mut self, key: RedisKeySummary, window: &mut Window, cx: &mut Context<Self>) {
        self.active_key = Some(key);
        self.load_value("0".into(), 0, false, window, cx);
    }

    /// Hashes and sets page by cursor, lists and sorted sets by index.
    fn load_more_value(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match self.value.as_ref().and_then(more_value) {
            Some(MoreValue::Cursor(cursor)) => self.load_value(cursor, 0, true, window, cx),
            Some(MoreValue::Offset(offset)) => {
                self.load_value("0".into(), offset, true, window, cx)
            }
            None => {}
        }
    }

    fn load_value(
        &mut self,
        cursor: String,
        offset: i64,
        append: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(key) = &self.active_key else {
            return;
        };
        self.value_loading = true;
        self.value_appending = append && self.value.is_some();
        self.value_error = None;
        self.value_seq += 1;
        let seq = self.value_seq;
        let core = self.ctx.core().clone();
        let (host_id, db) = (self.ctx.host_id, self.active_db);
        let key_b64 = key.key_b64.clone();
        self.value_task = Some(self.ctx.run(
            window,
            cx,
            async move {
                core.redis_key_value(
                    host_id,
                    db,
                    key_b64,
                    Some(cursor),
                    offset,
                    MAX_ELEMENTS_PER_PAGE,
                )
                .await
            },
            move |this, result, _, cx| {
                if seq != this.value_seq {
                    return;
                }
                this.value_loading = false;
                match result {
                    Ok(page) => {
                        let page = match this.value.take() {
                            Some(shown) if append => append_page(shown, page),
                            _ => page,
                        };
                        this.value_lines = if page.kind == "string" {
                            page.entries
                                .first()
                                .map(|e| display_lines(&e.value.text))
                                .unwrap_or_default()
                                .into_iter()
                                .map(SharedString::from)
                                .collect()
                        } else {
                            Vec::new()
                        };
                        this.value = Some(page);
                    }
                    Err(e) => this.value_error = Some(e),
                }
                this.value_appending = false;
                cx.notify();
            },
        ));
        cx.notify();
    }

    fn copy_value(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(text) = self
            .value
            .as_ref()
            .and_then(copyable_value)
            .map(str::to_owned)
        else {
            return;
        };
        cx.write_to_clipboard(ClipboardItem::new_string(text));
        self.ctx
            .toast(window, cx, "Copied to clipboard", ToastKind::Success);
    }

    // ---- backup and restore --------------------------------------------

    fn ask_backup(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy() || self.info.is_none() || self.is_cluster() {
            return;
        }
        let today = {
            use chrono::Datelike as _;
            let now = chrono::Local::now();
            (now.year(), now.month(), now.day())
        };
        let name = default_backup_name(
            &self.ctx.host_name(cx).unwrap_or_default(),
            self.active_db,
            &self.pattern,
            today,
        );
        let this = cx.entity().downgrade();
        self.ctx.save_file(
            window,
            cx,
            "Back up Redis database",
            &name,
            &[BACKUP_FILTER],
            move |path, window, cx| {
                let _ = this.update(cx, |this, cx| this.start_backup(path, window, cx));
            },
        );
    }

    /// Subscribe to a **fresh** op id before starting, so a cancel can only
    /// reach this run and no event can be missed.
    fn begin_op(&mut self, title: &str, cx: &mut Context<Self>) -> String {
        let op = self.ctx.new_op_id();
        self.op_sub = Some(self.ctx.subscribe_op(
            &op,
            cx.entity().downgrade(),
            |this: &mut Self, event, cx| {
                if this.op.as_mut().is_some_and(|op| op.apply(event)) {
                    cx.notify();
                }
            },
        ));
        self.op = Some(OpProgress::new(op.clone(), title));
        cx.notify();
        op
    }

    fn end_op(&mut self) {
        self.op = None;
        self.op_sub = None;
    }

    fn start_backup(
        &mut self,
        path: std::path::PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.busy() {
            return;
        }
        let op = self.begin_op("Backing up", cx);
        let core = self.ctx.core().clone();
        let events = self.ctx.events();
        let version = self.ctx.app_version();
        let (host_id, db) = (self.ctx.host_id, self.active_db);
        let pattern = Some(self.pattern.clone()).filter(|p| !p.is_empty());
        let output = path.to_string_lossy().into_owned();
        self.op_task = Some(self.ctx.run(
            window,
            cx,
            async move {
                core.redis_export(
                    events,
                    op,
                    host_id,
                    db,
                    pattern,
                    output,
                    Some(MAX_DUMP_KEY_BYTES),
                    version,
                )
                .await
            },
            |this, result, window, cx| this.op_done(result, "Backup", "Backed up", window, cx),
        ));
    }

    fn op_done(
        &mut self,
        result: Result<RedisOpSummary, String>,
        noun: &str,
        verb: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.end_op();
        let (message, kind) = match &result {
            Ok(summary) => summarize_op(summary, verb),
            Err(e) => op_error_message(e, noun),
        };
        // Only a success is announced on the desktop; a failure or a cancel
        // has its toast (`useRedisBackup.js`'s `finish`).
        if kind == ToastKind::Success {
            self.ctx
                .notify_if_unfocused(window, cx, format!("{noun} complete"), message.clone());
        }
        self.ctx.toast(window, cx, message, kind);
        cx.notify();
    }

    fn ask_restore(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy() || self.info.is_none() {
            return;
        }
        let this = cx.entity().downgrade();
        self.ctx.pick_file(
            window,
            cx,
            "Restore a Redis backup",
            &[BACKUP_FILTER],
            move |path, window, cx| {
                let _ = this.update(cx, |this, cx| this.read_backup(path, window, cx));
            },
        );
    }

    /// Read the file's header so the dialog can say what is about to happen.
    /// A file that fails validation is reported, never retried.
    fn read_backup(
        &mut self,
        path: std::path::PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let path = path.to_string_lossy().into_owned();
        let read_path = path.clone();
        let job = self
            .ctx
            .spawn_blocking(move || termdrop_core::redis::read_backup_header(&read_path));
        self.header_task = Some(self.ctx.run(
            window,
            cx,
            async move {
                job.await
                    .unwrap_or_else(|e| Err(format!("task failed: {e}")))
            },
            move |this, result, window, cx| match result {
                Err(e) => this.ctx.toast(
                    window,
                    cx,
                    format!("That file cannot be restored: {e}"),
                    ToastKind::Error,
                ),
                Ok(header) => {
                    let version = this.info.as_ref().map_or("", |i| i.version.as_str());
                    let warning = restore_warning(&header, version);
                    let target = default_restore_db(&header, this.active_db);
                    let name = this.ctx.host_name(cx).unwrap_or_default();
                    let tab = cx.entity().downgrade();
                    open_restore_dialog(
                        window,
                        cx,
                        name,
                        path,
                        header,
                        warning,
                        target,
                        move |choice, window, cx| {
                            let _ =
                                tab.update(cx, |this, cx| this.start_restore(choice, window, cx));
                        },
                    );
                }
            },
        ));
    }

    fn start_restore(
        &mut self,
        choice: RestoreChoice,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.busy() {
            return;
        }
        let op = self.begin_op("Restoring", cx);
        let core = self.ctx.core().clone();
        let events = self.ctx.events();
        let host_id = self.ctx.host_id;
        self.op_task = Some(self.ctx.run(
            window,
            cx,
            async move {
                core.redis_import(
                    events,
                    op,
                    host_id,
                    choice.target_db,
                    choice.path,
                    choice.replace,
                    choice.flush_first,
                )
                .await
            },
            |this, result, window, cx| {
                let ok = result.is_ok();
                this.op_done(result, "Restore", "Restored", window, cx);
                if ok {
                    this.refresh_all(window, cx);
                }
            },
        ));
    }

    fn cancel_op(&mut self, cx: &mut Context<Self>) {
        if let Some(op) = self.op.as_mut().filter(|op| op.is_active()) {
            op.cancelling();
            self.ctx.cancel_op(&op.op_id);
            cx.notify();
        }
    }

    // ---- rendering: header, banners ------------------------------------

    fn render_header(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let busy = self.busy();
        let cluster = self.is_cluster();
        let connected = self.info.is_some();
        let mut header = PanelHeader::new(self.ctx.host_name(cx).unwrap_or_else(|| "Redis".into()))
            .subtitle(self.display_uri.clone())
            .subtitle_mono()
            .meta(
                self.info
                    .as_ref()
                    .map(|i| format!("Redis {}", i.version))
                    .unwrap_or_default(),
            );
        if self.info.as_ref().is_some_and(|i| i.tunnelled) {
            let tip = format!("Tunnelled through {}", self.tunnel_name);
            header = header.badge(
                Tag::new("redis-via", format!("via {}", self.tunnel_name))
                    .tone(TagTone::Accent)
                    .tooltip(tip),
            );
        }
        if cluster {
            header = header.badge(Tag::new("redis-cluster", "cluster").tone(TagTone::Warn).tooltip(
                "SCAN sees only this node, so the key list is one node's keyspace and backup \
                 is disabled",
            ));
        }
        let mut backup = Button::new("redis-backup")
            .ghost()
            .xsmall()
            .label("Back up")
            .disabled(busy || !connected || cluster)
            .on_click(cx.listener(|this, _, window, cx| this.ask_backup(window, cx)));
        if cluster {
            backup = backup.tooltip("A cluster backup would cover only this node");
        }
        header
            .action(
                IconButton::new("redis-refresh", IconName::RefreshCw, "Refresh")
                    .disabled(busy)
                    .pending(self.refreshing)
                    .on_click(cx.listener(|this, _, window, cx| this.refresh_all(window, cx))),
            )
            .action(backup)
            .action(
                Button::new("redis-restore")
                    .ghost()
                    .xsmall()
                    .label("Restore")
                    .disabled(busy || !connected)
                    .on_click(cx.listener(|this, _, window, cx| this.ask_restore(window, cx))),
            )
    }

    fn render_connect_error(&self, error: &str, p: &Palette, cx: &mut Context<Self>) -> AnyElement {
        h_flex()
            .flex_none()
            .items_start()
            .gap_2()
            .px_3()
            .py_2()
            .bg(p.bad_bg)
            .border_b_1()
            .border_color(p.bad_line)
            .child(
                div()
                    .flex_none()
                    .pt_0p5()
                    .child(Icon::new(IconName::CircleAlert).small().text_color(p.bad)),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .text_xs()
                    .text_color(p.bad)
                    .child(SharedString::from(error.to_string())),
            )
            .child(
                Button::new("redis-reconnect")
                    .ghost()
                    .xsmall()
                    .label("Reconnect")
                    .disabled(self.connecting)
                    .on_click(cx.listener(|this, _, window, cx| this.connect(window, cx))),
            )
            .into_any_element()
    }

    // ---- rendering: the tree -------------------------------------------

    fn current_tree_rows(&self) -> Vec<TreeRow> {
        tree_rows(
            &self.databases(),
            self.active_db,
            &self.tree.groups,
            self.tree_loading,
            self.tree.truncated,
            self.tree.scanned,
        )
    }

    fn render_tree(&self, p: &Palette, cx: &mut Context<Self>) -> impl IntoElement {
        let count = self.current_tree_rows().len();
        div()
            .flex_none()
            .w(px(TREE_W))
            .h_full()
            .border_r_1()
            .border_color(p.line)
            .py_1()
            .child(
                uniform_list("redis-tree", count, cx.processor(Self::render_tree_rows)).size_full(),
            )
    }

    fn render_tree_rows(
        &mut self,
        range: std::ops::Range<usize>,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        let p = *palette(cx);
        let rows = self.current_tree_rows();
        range
            .filter_map(|i| rows.get(i).cloned().map(|row| (i, row)))
            .map(|(i, row)| {
                let base = h_flex()
                    .id(SharedString::from(format!("redis-tree-{i}")))
                    .w_full()
                    .h(px(ROW_H))
                    .gap_1p5()
                    .text_xs();
                match row {
                    TreeRow::Db {
                        index,
                        keys,
                        active,
                    } => {
                        base.px_2()
                            .cursor_pointer()
                            .text_color(p.ink)
                            .when(active, |d| d.bg(p.selected))
                            .hover(move |s| if active { s } else { s.bg(p.raised) })
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.select_db(index, window, cx)
                            }))
                            .child(
                                Icon::new(if active {
                                    IconName::ChevronDown
                                } else {
                                    IconName::ChevronRight
                                })
                                .xsmall()
                                .text_color(p.ink_2),
                            )
                            .child(Icon::new(IconName::Layers).xsmall().text_color(p.redis))
                            .child(div().flex_1().truncate().child(format!("db{index}")))
                            .child(
                                text_2xs(div())
                                    .text_color(p.ink_3)
                                    .child(format_count(keys)),
                            )
                            .into_any_element()
                    }
                    TreeRow::Grouping => base
                        .pl(px(24.))
                        .pr_2()
                        .text_color(p.ink_2)
                        .child(Spinner::new().xsmall().color(p.ink_2))
                        .child("Grouping keys\u{2026}")
                        .into_any_element(),
                    TreeRow::Group(ix) => {
                        let Some(group) = self.tree.groups.get(ix) else {
                            return base.into_any_element();
                        };
                        let selected = !group.binary
                            && self.active_group.as_deref() == Some(group.prefix.as_str());
                        let tip = if group.binary {
                            "binary prefix".to_string()
                        } else {
                            group.prefix.clone()
                        };
                        base.pl(px(28.))
                            .pr_2()
                            .cursor_pointer()
                            .text_color(if selected { p.ink } else { p.ink_2 })
                            .when(selected, |d| d.bg(p.selected))
                            .hover(move |s| if selected { s } else { s.bg(p.raised) })
                            .tooltip(move |window, cx| Tooltip::new(tip.clone()).build(window, cx))
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.select_group(ix, window, cx)
                            }))
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .truncate()
                                    .child(group_label(group)),
                            )
                            .child(
                                text_2xs(div())
                                    .flex_none()
                                    .text_color(p.ink_3)
                                    .child(format_count(group.count)),
                            )
                            .into_any_element()
                    }
                    // The scan is capped, so the groups may describe part of
                    // the keyspace. Saying so is the point: a tree implying
                    // it was complete would be quietly wrong on a real server.
                    TreeRow::Partial { scanned } => base
                        .pl(px(28.))
                        .pr_2()
                        .child(text_2xs(div()).italic().text_color(p.ink_3).child(format!(
                            "\u{22ef} partial: grouped the first {} keys",
                            format_count(scanned)
                        )))
                        .into_any_element(),
                    TreeRow::NoMatch => base
                        .pl(px(28.))
                        .pr_2()
                        .child(
                            text_2xs(div())
                                .italic()
                                .text_color(p.ink_3)
                                .child("No keys match"),
                        )
                        .into_any_element(),
                }
            })
            .collect()
    }

    // ---- rendering: the key list ---------------------------------------

    fn render_keys(&self, p: &Palette, cx: &mut Context<Self>) -> impl IntoElement {
        let filter_bar = h_flex()
            .flex_none()
            .gap_2()
            .px_3()
            .py_2()
            .border_b_1()
            .border_color(p.line)
            .child(
                div().flex_1().min_w_0().child(
                    Input::new(&self.pattern_input)
                        .xsmall()
                        .prefix(Icon::new(IconName::Search).xsmall().text_color(p.ink_3)),
                ),
            )
            .child(
                div()
                    .flex_none()
                    .w(px(120.))
                    .child(Select::new(&self.type_select).xsmall().w_full()),
            )
            .child(
                Button::new("redis-search")
                    .primary()
                    .xsmall()
                    .label("Search")
                    .on_click(cx.listener(|this, _, window, cx| this.on_search(window, cx))),
            );

        let body: AnyElement = if self.keys_loading {
            EmptyState::loading("redis-keys-loading", "Scanning\u{2026}")
                .compact()
                .into_any_element()
        } else if self.keys.is_empty() {
            let empty = empty_page(&self.pattern, &self.type_filter, self.keys_done);
            let action = empty.action;
            let on_action = cx.listener(move |this, _, window, cx| match action {
                EmptyAction::NextPage => this.next_page(window, cx),
                EmptyAction::ClearFilter => this.clear_filter(window, cx),
                EmptyAction::Refresh => this.restart_keys(window, cx),
            });
            let state = if empty.filtered {
                EmptyState::filtered("redis-keys-empty", empty.title, action.label(), on_action)
            } else {
                EmptyState::empty("redis-keys-empty", empty.title, action.label(), on_action)
            };
            state
                .icon(Icon::new(IconName::Search))
                .when_some(empty.hint, |s, hint| s.hint(hint))
                .compact()
                .into_any_element()
        } else {
            v_flex()
                .size_full()
                .child(
                    list_head(h_flex().flex_none().w_full(), p)
                        .child(div().flex_1().min_w_0().child("KEY"))
                        .child(div().flex_none().w(px(TYPE_W)).child("TYPE"))
                        .child(div().flex_none().w(px(TTL_W)).child("TTL"))
                        .when(self.with_memory, |row| {
                            row.child(
                                div()
                                    .flex_none()
                                    .w(px(SIZE_W))
                                    .flex()
                                    .justify_end()
                                    .child("SIZE"),
                            )
                        }),
                )
                .child(
                    uniform_list(
                        "redis-keys",
                        self.keys.len(),
                        cx.processor(Self::render_key_rows),
                    )
                    .flex_1()
                    .min_h_0(),
                )
                .into_any_element()
        };

        let has_next = !self.keys_done && !self.keys_loading;
        let footer = h_flex()
            .flex_none()
            .justify_between()
            .gap_2()
            .px_3()
            .py(px(6.))
            .border_t_1()
            .border_color(p.line)
            .text_xs()
            .text_color(p.ink_2)
            .child(div().min_w_0().truncate().child(summarize_scan(
                self.keys.len(),
                db_key_count(&self.databases(), self.active_db),
                self.keys_done && self.page_number == 1,
            )))
            .child(
                h_flex()
                    .flex_none()
                    .gap_2()
                    .child(
                        Checkbox::new("redis-sizes")
                            .xsmall()
                            .label("Sizes")
                            .tooltip("Costs an extra command per key")
                            .checked(self.with_memory)
                            .on_click(cx.listener(|this, checked: &bool, window, cx| {
                                this.set_with_memory(*checked, window, cx)
                            })),
                    )
                    .child(
                        Button::new("redis-prev")
                            .ghost()
                            .xsmall()
                            .label("\u{2039} prev")
                            .disabled(!self.can_go_back || self.keys_loading)
                            .on_click(
                                cx.listener(|this, _, window, cx| this.prev_page(window, cx)),
                            ),
                    )
                    .child(format!("page {}", self.page_number))
                    .child(
                        Button::new("redis-next")
                            .ghost()
                            .xsmall()
                            .label("next \u{203a}")
                            .disabled(!has_next)
                            .on_click(
                                cx.listener(|this, _, window, cx| this.next_page(window, cx)),
                            ),
                    ),
            );

        v_flex()
            .flex_1()
            .min_w_0()
            .h_full()
            .child(filter_bar)
            .when_some(self.keys_error.clone(), |col, error| {
                col.child(
                    div()
                        .flex_none()
                        .px_3()
                        .py_2()
                        .text_xs()
                        .text_color(p.bad)
                        .border_b_1()
                        .border_color(p.line)
                        .child(error),
                )
            })
            .child(div().flex_1().min_h_0().child(body))
            .child(footer)
    }

    fn render_key_rows(
        &mut self,
        range: std::ops::Range<usize>,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        let p = *palette(cx);
        let mono = mono_font(cx);
        let active = self.active_key.as_ref().map(|k| k.key_b64.clone());
        range
            .filter_map(|i| self.keys.get(i).cloned().map(|k| (i, k)))
            .map(|(i, key)| {
                let selected = active.as_deref() == Some(key.key_b64.as_str());
                let label = key_label(&key.key);
                let binary = key.key.binary;
                let row = h_flex()
                    .id(SharedString::from(format!("redis-key-{i}")))
                    .w_full()
                    .h(px(ROW_H))
                    .px_2()
                    .text_xs();
                let open = key.clone();
                list_row(row, &p, selected, true)
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.open_key(open.clone(), window, cx)
                    }))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .font_family(mono.clone())
                            .text_color(if binary { p.warn } else { p.ink })
                            .child(label),
                    )
                    .child(
                        div()
                            .flex_none()
                            .w(px(TYPE_W))
                            .truncate()
                            .text_color(p.ink_2)
                            .child(format_key_kind(&key.kind)),
                    )
                    .child(
                        div()
                            .flex_none()
                            .w(px(TTL_W))
                            .truncate()
                            .text_color(p.ink_2)
                            .child(format_ttl(key.ttl_ms)),
                    )
                    .when(self.with_memory, |row| {
                        row.child(
                            div()
                                .flex_none()
                                .w(px(SIZE_W))
                                .flex()
                                .justify_end()
                                .text_color(p.ink_2)
                                .child(
                                    key.memory_bytes
                                        .map_or_else(|| "\u{2014}".to_string(), format_bytes),
                                ),
                        )
                    })
                    .into_any_element()
            })
            .collect()
    }

    // ---- rendering: the value ------------------------------------------

    fn render_value(&self, p: &Palette, cx: &mut Context<Self>) -> impl IntoElement {
        let pane = v_flex()
            .flex_none()
            .w(px(VALUE_W))
            .h_full()
            .min_w_0()
            .border_l_1()
            .border_color(p.line);
        let Some(key) = &self.active_key else {
            return pane
                .child(
                    EmptyState::notice("redis-value-none", "Select a key to view it")
                        .icon(Icon::new(IconName::FileText))
                        .compact(),
                )
                .into_any_element();
        };
        let mono = mono_font(cx);
        let page = self.value.as_ref();

        let mut meta = h_flex().gap_3().mt_1().text_color(p.ink_2);
        if let Some(page) = page {
            meta = meta
                .child(format_key_kind(&page.kind))
                .child(format_ttl(page.ttl_ms))
                .when_some(page.encoding.clone(), |m, enc| m.child(enc))
                .when_some(total_label(page), |m, total| m.child(total));
        } else {
            meta = meta.child(format_key_kind(&key.kind));
        }
        // Only a whole text value: a truncated preview or base64 would
        // reach the clipboard as if it were the value.
        let can_copy = page.is_some_and(|v| copyable_value(v).is_some());
        let head = h_flex()
            .flex_none()
            .items_start()
            .gap_2()
            .px_3()
            .py_2()
            .border_b_1()
            .border_color(p.line)
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .child(
                        div()
                            .font_family(mono.clone())
                            .text_xs()
                            .text_color(p.ink)
                            .child(key_label(&key.key)),
                    )
                    .child(text_2xs(meta)),
            )
            .when(can_copy && !self.value_loading, |h| {
                h.child(
                    IconButton::new("redis-copy-value", IconName::Copy, "Copy value")
                        .on_click(cx.listener(|this, _, window, cx| this.copy_value(window, cx))),
                )
            });

        let body: AnyElement = if self.value_loading && !self.value_appending {
            EmptyState::loading("redis-value-loading", "Reading\u{2026}")
                .compact()
                .into_any_element()
        } else if let Some(error) = &self.value_error {
            let retry = key.clone();
            EmptyState::error(
                "redis-value-error",
                "Could not read this key",
                "Retry",
                cx.listener(move |this, _, window, cx| this.open_key(retry.clone(), window, cx)),
            )
            .hint(error.clone())
            .compact()
            .into_any_element()
        } else if let Some(page) = page {
            // A vanished key and an undisplayable type are different
            // problems: one is an error, the other simply is.
            if page.kind == "none" {
                EmptyState::error(
                    "redis-value-gone",
                    "This key no longer exists",
                    "Refresh list",
                    cx.listener(|this, _, window, cx| this.restart_keys(window, cx)),
                )
                .hint("It expired or was deleted since the list was built")
                .compact()
                .into_any_element()
            } else if !is_viewable_kind(&page.kind) {
                EmptyState::notice(
                    "redis-value-unviewable",
                    "TermDrop cannot display this type",
                )
                .icon(Icon::new(IconName::CircleAlert))
                .hint(format!(
                    "{} \u{2014} usually a Redis module type",
                    page.kind
                ))
                .compact()
                .into_any_element()
            } else {
                self.render_entries(page, p, mono, cx)
            }
        } else {
            div().into_any_element()
        };

        let footer = page
            .filter(|v| is_viewable_kind(&v.kind) && v.kind != "string")
            .filter(|_| self.value_error.is_none())
            .map(|v| {
                h_flex()
                    .flex_none()
                    .justify_between()
                    .px_3()
                    .py(px(6.))
                    .border_t_1()
                    .border_color(p.line)
                    .text_xs()
                    .text_color(p.ink_2)
                    .child(format!("{} shown", format_count(v.entries.len() as u64)))
                    .when(more_value(v).is_some(), |f| {
                        f.child(
                            Button::new("redis-value-more")
                                .ghost()
                                .xsmall()
                                .label("Load more")
                                .loading(self.value_appending)
                                .disabled(self.value_loading)
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.load_more_value(window, cx)
                                })),
                        )
                    })
            });

        pane.child(head)
            .child(div().flex_1().min_h_0().flex().flex_col().child(body))
            .children(footer)
            .into_any_element()
    }

    /// A string's lines, or a table of elements; the note sits above, never
    /// under, what it describes.
    fn render_entries(
        &self,
        page: &RedisValuePage,
        p: &Palette,
        mono: SharedString,
        _: &mut Context<Self>,
    ) -> AnyElement {
        let note = truncated_note(page).map(|note| {
            text_2xs(div())
                .flex_none()
                .px_3()
                .py(px(6.))
                .text_color(p.warn)
                .bg(p.warn_bg)
                .border_b_1()
                .border_color(p.warn_line)
                .child(note)
        });
        let scroll = v_flex()
            .id("redis-value-scroll")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .font_family(mono.clone())
            .text_xs();
        let content: AnyElement = if page.kind == "string" {
            scroll
                .px_3()
                .py_2()
                .text_color(p.ink)
                .children(self.value_lines.iter().map(|line| {
                    // An empty line still takes its height.
                    div().min_h(px(16.)).child(line.clone())
                }))
                .into_any_element()
        } else {
            let field = field_header(&page.kind);
            let score = page.kind == "zset";
            let head = list_head(h_flex().flex_none().w_full(), p)
                .px_3()
                .when_some(field, |h, f| {
                    h.child(
                        div()
                            .flex_none()
                            .w(px(VALUE_W / 3.))
                            .child(f.to_uppercase()),
                    )
                })
                .child(div().flex_1().child("VALUE"))
                .when(score, |h| {
                    h.child(
                        div()
                            .flex_none()
                            .w(px(96.))
                            .flex()
                            .justify_end()
                            .child("SCORE"),
                    )
                });
            let rows = page.entries.iter().map(|entry| {
                h_flex()
                    .items_start()
                    .w_full()
                    .px_3()
                    .py_1()
                    .gap_2()
                    .border_t_1()
                    .border_color(p.line.opacity(0.3))
                    .when(field.is_some(), |r| {
                        r.child(
                            div()
                                .flex_none()
                                .w(px(VALUE_W / 3. - 8.))
                                .min_w_0()
                                .text_color(p.accent)
                                .child(cell(
                                    entry.field.as_ref().map_or("", |f| f.text.as_str()),
                                    p,
                                )),
                        )
                    })
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_color(p.ink)
                            .child(cell(&entry.value.text, p)),
                    )
                    .when(score, |r| {
                        r.child(
                            div()
                                .flex_none()
                                .w(px(96.))
                                .flex()
                                .justify_end()
                                .text_color(p.ink_2)
                                .child(entry.score.map(|s| s.to_string()).unwrap_or_default()),
                        )
                    })
            });
            v_flex()
                .flex_1()
                .min_h_0()
                .child(head)
                .child(scroll.children(rows))
                .into_any_element()
        };
        v_flex()
            .size_full()
            .children(note)
            .child(content)
            .into_any_element()
    }
}

impl Render for RedisTab {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = *palette(cx);
        let this = cx.entity().downgrade();
        let progress = self.op.as_ref().filter(|op| op.is_active()).map(|op| {
            div()
                .flex_none()
                .border_b_1()
                .border_color(p.line)
                .child(op_progress_row("redis-op", op, &p, move |_, cx| {
                    let _ = this.update(cx, |this, cx| this.cancel_op(cx));
                }))
        });
        let body: AnyElement = if self.connecting {
            EmptyState::loading("redis-connecting", "Connecting\u{2026}").into_any_element()
        } else if self.info.is_some() {
            h_flex()
                .size_full()
                .items_start()
                .child(self.render_tree(&p, cx))
                .child(self.render_keys(&p, cx))
                .child(self.render_value(&p, cx))
                .into_any_element()
        } else {
            div().into_any_element()
        };
        v_flex()
            .size_full()
            .bg(p.canvas)
            .child(self.render_header(cx))
            .children(progress)
            .when_some(self.connect_error.clone(), |col, error| {
                col.child(self.render_connect_error(&error, &p, cx))
            })
            .child(div().flex_1().min_h_0().flex().flex_col().child(body))
    }
}

fn empty_tree() -> KeyTree {
    KeyTree {
        groups: Vec::new(),
        scanned: 0,
        truncated: false,
    }
}

/// One table cell's text, drawn at most [`logic::MAX_CELL_BYTES`] long with
/// what was left out said after it.
fn cell(text: &str, p: &Palette) -> AnyElement {
    let (shown, hidden) = clip_cell(text);
    div()
        .child(SharedString::from(shown.to_string()))
        .when(hidden > 0, |d| {
            d.child(
                div()
                    .text_color(p.ink_3)
                    .child(format!("\u{2026} {} more", format_bytes(hidden as u64))),
            )
        })
        .into_any_element()
}
