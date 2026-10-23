//! The MongoDB tab: port of `MongodbPanel.vue`, `DbTree.vue`,
//! `MongoDocumentsView.vue` (`documents.rs`), `MongoRestoreDialog.vue`
//! (`restore.rs`) and the composables `useMongoConnection.js` and
//! `useMongoBackup.js`. Built against the contract in `../README.md`.
//!
//! The rules and wording are in the pure modules, which are tested:
//! `bson.rs` (extended JSON for display, table columns), `query.rs` (filter,
//! paging, the stats line) and `selection.rs` (ticked collections, the
//! dump/restore runs and what each says). This file lays them out and runs
//! the core calls.
//!
//! One difference from the Vue tab: closing the tab cancels a backup or
//! restore that is still running. In the Vue app it carried on with no row
//! left to show its progress or cancel it.

mod bson;
mod documents;
mod query;
mod restore;
mod selection;

use std::collections::{HashMap, HashSet, VecDeque};

use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{Disableable as _, Icon, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, AnyView, App, AppContext as _, Context, Entity, FontWeight, InteractiveElement,
    IntoElement, ParentElement, Render, SharedString, StatefulInteractiveElement as _, Styled,
    Subscription, Task, Window, div, px,
};
use termdrop_core::mongodb::scan_restore_folder;

use self::documents::DocumentsView;
use self::restore::{RestoreRequest, confirm_restore};
use self::selection::{
    ARCHIVE_ONE_DB, CheckState, DbEntry, Job, JobKind, Selection, archive_file_name,
    archive_restore_job, backup_jobs, backup_tooltip, folder_restore_jobs, is_cancel,
};
use super::common::{OpProgress, completed_summary, op_progress_row, text_2xs, tri_check};
use super::{OpSubscription, ServiceTabContext};
use crate::ui::empty_state::EmptyState;
use crate::ui::file_dialog::ALL_FILES;
use crate::ui::icon_button::IconButton;
use crate::ui::panel_header::PanelHeader;
use crate::ui::theme::{Palette, palette};
use crate::ui::toast::ToastKind;

/// The tree's width (`w-72`).
const TREE_W: f32 = 288.;

/// The tab's view, built once when the tab opens and dropped when it closes.
pub fn build(ctx: ServiceTabContext, window: &mut Window, cx: &mut App) -> AnyView {
    cx.new(|cx| MongoTab::new(ctx, window, cx)).into()
}

/// A backup or restore in progress: its remaining runs, one at a time,
/// each with a fresh op id (`with_mongo_op` registers per call, so a reused
/// id would leave Cancel nothing to find between runs).
struct Run {
    jobs: VecDeque<Job>,
    progress: OpProgress,
    aborting: bool,
    /// Reload the tree when the run ends (after a restore).
    refresh_after: bool,
    /// What each job that succeeded ran on, for the one desktop notice at
    /// the end (`notifyCompleted`).
    done: Vec<String>,
    _sub: Option<OpSubscription>,
    _task: Option<Task<()>>,
}

pub struct MongoTab {
    ctx: ServiceTabContext,
    databases: Vec<DbEntry>,
    expanded: HashSet<String>,
    loading: bool,
    list_error: Option<String>,
    selection: Selection,
    /// Databases whose box was ticked before their collections had loaded.
    select_when_loaded: HashSet<String>,
    viewing: Option<(String, String)>,
    documents: Option<Entity<DocumentsView>>,
    run: Option<Run>,
    list_task: Option<Task<()>>,
    collection_tasks: HashMap<String, Task<()>>,
    scan_task: Option<Task<()>>,
    _subscriptions: Vec<Subscription>,
}

impl MongoTab {
    fn new(ctx: ServiceTabContext, window: &mut Window, cx: &mut Context<Self>) -> Self {
        // A closed tab leaves no row to show or cancel a running backup, so
        // it stops with the tab.
        let release = cx.on_release(|this: &mut Self, _| {
            if let Some(run) = &this.run {
                this.ctx.cancel_op(&run.progress.op_id);
            }
        });
        let mut tab = Self {
            ctx,
            databases: Vec::new(),
            expanded: HashSet::new(),
            loading: false,
            list_error: None,
            selection: Selection::default(),
            select_when_loaded: HashSet::new(),
            viewing: None,
            documents: None,
            run: None,
            list_task: None,
            collection_tasks: HashMap::new(),
            scan_task: None,
            _subscriptions: vec![release],
        };
        tab.load_databases(window, cx);
        tab
    }

    fn name(&self, cx: &App) -> String {
        self.ctx
            .host_name(cx)
            .filter(|n| !n.is_empty())
            .unwrap_or_else(|| "MongoDB".into())
    }

    fn busy(&self) -> bool {
        self.run.is_some()
    }

    // ---- the tree ------------------------------------------------------

    fn load_databases(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.loading = true;
        self.list_error = None;
        cx.notify();
        let host = self.ctx.host_id;
        self.list_task = Some(self.ctx.run_with_secret(
            window,
            cx,
            move |core, _| async move { core.mongodb_list_databases(host).await },
            |this, result, window, cx| {
                this.loading = false;
                match result {
                    Ok(names) => {
                        this.databases = names.into_iter().map(DbEntry::new).collect();
                        // Expanded databases keep showing their collections.
                        let open: Vec<String> = this
                            .databases
                            .iter()
                            .filter(|d| this.expanded.contains(&d.name))
                            .map(|d| d.name.clone())
                            .collect();
                        this.expanded.retain(|n| open.contains(n));
                        for name in open {
                            this.fetch_collections(&name, window, cx);
                        }
                    }
                    Err(e) => {
                        this.ctx.toast(
                            window,
                            cx,
                            format!("Failed to list databases: {e}"),
                            ToastKind::Error,
                        );
                        this.list_error = Some(e);
                    }
                }
                cx.notify();
            },
        ));
    }

    fn fetch_collections(&mut self, db: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(entry) = self.databases.iter_mut().find(|d| d.name == db) else {
            return;
        };
        entry.loading = true;
        let name = db.to_string();
        let call_db = name.clone();
        let host = self.ctx.host_id;
        let task = self.ctx.run_with_secret(
            window,
            cx,
            move |core, _| {
                let db = call_db.clone();
                async move { core.mongodb_list_collections(host, db).await }
            },
            move |this, result, window, cx| {
                this.collection_tasks.remove(&name);
                let Some(entry) = this.databases.iter_mut().find(|d| d.name == name) else {
                    return;
                };
                entry.loading = false;
                match result {
                    Ok(colls) => {
                        entry.collections = colls;
                        entry.loaded = true;
                        if this.select_when_loaded.remove(&name) {
                            let entry = entry.clone();
                            this.selection.select_all(&entry);
                        }
                    }
                    Err(e) => {
                        this.select_when_loaded.remove(&name);
                        this.ctx.toast(
                            window,
                            cx,
                            format!("Failed to list collections for {name}: {e}"),
                            ToastKind::Error,
                        );
                    }
                }
                cx.notify();
            },
        );
        self.collection_tasks.insert(db.to_string(), task);
    }

    fn toggle_db(&mut self, db: &str, window: &mut Window, cx: &mut Context<Self>) {
        if !self.expanded.remove(db) {
            self.expanded.insert(db.to_string());
            if self
                .databases
                .iter()
                .any(|d| d.name == db && !d.loaded && !d.loading)
            {
                self.fetch_collections(db, window, cx);
            }
        }
        cx.notify();
    }

    fn toggle_db_selection(&mut self, db: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(entry) = self.databases.iter().find(|d| d.name == db).cloned() else {
            return;
        };
        if self.selection.db_state(&entry) == CheckState::All {
            self.selection.remove_db(db);
        } else if entry.loaded {
            self.selection.select_all(&entry);
        } else {
            // Selecting a whole database needs its collections, which load
            // lazily.
            self.select_when_loaded.insert(db.to_string());
            self.expanded.insert(db.to_string());
            if !entry.loading {
                self.fetch_collections(db, window, cx);
            }
        }
        cx.notify();
    }

    fn open_collection(
        &mut self,
        db: &str,
        coll: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let key = (db.to_string(), coll.to_string());
        if self.viewing.as_ref() == Some(&key) {
            return;
        }
        let ctx = self.ctx.clone();
        let (d, c) = key.clone();
        self.documents = Some(cx.new(|cx| DocumentsView::new(ctx, d, c, window, cx)));
        self.viewing = Some(key);
        cx.notify();
    }

    // ---- backup and restore --------------------------------------------

    fn can_backup(&self) -> bool {
        !self.selection.is_empty() && !self.busy()
    }

    fn backup_folder(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.can_backup() {
            return;
        }
        let view = cx.entity().downgrade();
        self.ctx.pick_folder(
            window,
            cx,
            "Select backup folder",
            move |dir, window, cx| {
                let _ = view.update(cx, |this, cx| {
                    let entries = this.selection.entries();
                    let jobs = backup_jobs(&entries, &dir.to_string_lossy(), false);
                    this.start_run(jobs, false, window, cx);
                });
            },
        );
    }

    fn backup_archive(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.can_backup() {
            return;
        }
        let dbs = self.selection.dbs();
        if dbs.len() > 1 {
            self.ctx.toast(window, cx, ARCHIVE_ONE_DB, ToastKind::Error);
            return;
        }
        let view = cx.entity().downgrade();
        self.ctx.save_file(
            window,
            cx,
            "Save backup archive",
            &archive_file_name(&dbs),
            &[
                ("Gzip archive", &["gz"]),
                ("BSON archive", &["archive", "bson"]),
                ALL_FILES,
            ],
            move |file, window, cx| {
                let _ = view.update(cx, |this, cx| {
                    let entries = this.selection.entries();
                    let jobs = backup_jobs(&entries, &file.to_string_lossy(), true);
                    this.start_run(jobs, false, window, cx);
                });
            },
        );
    }

    fn restore_folder(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy() {
            return;
        }
        let view = cx.entity().downgrade();
        self.ctx.pick_folder(
            window,
            cx,
            "Select backup folder (mongodump output)",
            move |dir, window, cx| {
                let _ = view.update(cx, |this, cx| {
                    this.scan_folder(dir.to_string_lossy().into(), window, cx)
                });
            },
        );
    }

    fn scan_folder(&mut self, dir: String, window: &mut Window, cx: &mut Context<Self>) {
        let job = self.ctx.spawn_blocking({
            let dir = dir.clone();
            move || scan_restore_folder(dir)
        });
        self.scan_task = Some(cx.spawn_in(window, async move |this, cx| {
            let result = job
                .await
                .unwrap_or_else(|e| Err(format!("task failed: {e}")));
            let _ = cx.update(|window, cx| {
                this.update(cx, |this, cx| match result {
                    Ok(found) => {
                        let source = found
                            .into_iter()
                            .map(|d| (d.name, d.collections.into_iter().map(|c| c.name).collect()))
                            .collect();
                        this.confirm(dir, false, source, window, cx);
                    }
                    Err(e) => this.ctx.toast(
                        window,
                        cx,
                        format!("Selected folder is not a valid backup: {e}"),
                        ToastKind::Error,
                    ),
                })
            });
        }));
    }

    fn restore_file(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy() {
            return;
        }
        let view = cx.entity().downgrade();
        self.ctx.pick_file(
            window,
            cx,
            "Select backup archive",
            &[("MongoDB archives", &["gz", "archive", "bson"]), ALL_FILES],
            move |file, window, cx| {
                let _ = view.update(cx, |this, cx| {
                    this.confirm(file.to_string_lossy().into(), true, Vec::new(), window, cx)
                });
            },
        );
    }

    fn confirm(
        &mut self,
        input_path: String,
        is_archive: bool,
        source: Vec<(String, Vec<String>)>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let request = RestoreRequest {
            input_path,
            is_archive,
            entries: self.selection.entries(),
            source,
        };
        let view = cx.entity().downgrade();
        let name = self.name(cx);
        confirm_restore(
            window,
            cx,
            name,
            request,
            move |req, drop_first, window, cx| {
                let _ = view.update(cx, |this, cx| {
                    this.start_restore(req, drop_first, window, cx)
                });
            },
        );
    }

    fn start_restore(
        &mut self,
        req: RestoreRequest,
        drop_first: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.busy() {
            return;
        }
        let mut jobs = if req.is_archive {
            vec![archive_restore_job(&req.entries, &req.input_path)]
        } else {
            folder_restore_jobs(&req.entries, &req.source_dbs(), &req.input_path)
        };
        if drop_first {
            for job in &mut jobs {
                job.drop_first = true;
            }
        }
        self.start_run(jobs, true, window, cx);
    }

    fn start_run(
        &mut self,
        jobs: Vec<Job>,
        refresh_after: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if jobs.is_empty() || self.busy() {
            return;
        }
        self.run = Some(Run {
            jobs: jobs.into(),
            progress: OpProgress::new("", ""),
            aborting: false,
            refresh_after,
            done: Vec::new(),
            _sub: None,
            _task: None,
        });
        self.next_job(window, cx);
    }

    fn next_job(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(run) = self.run.as_mut() else {
            return;
        };
        let job = match run.jobs.pop_front() {
            Some(job) if !run.aborting => job,
            _ => return self.finish_run(window, cx),
        };
        let op = self.ctx.new_op_id();
        // Subscribed before the call starts, so no event is missed.
        run._sub = Some(self.ctx.subscribe_op(
            &op,
            cx.entity().downgrade(),
            |this: &mut Self, ev, cx| {
                if let Some(run) = this.run.as_mut()
                    && run.progress.apply(ev)
                {
                    cx.notify();
                }
            },
        ));
        run.progress = OpProgress::new(op.clone(), job.title.clone());

        let core = self.ctx.core().clone();
        let events = self.ctx.events();
        let host = self.ctx.host_id;
        let drop_first = job.drop_first;
        let kind = job.kind.clone();
        let future = async move {
            match kind {
                JobKind::Dump {
                    db,
                    collections,
                    output,
                    is_archive,
                } => {
                    core.mongodb_dump(events, op, host, db, collections, output, is_archive)
                        .await
                }
                JobKind::Restore {
                    db,
                    collections,
                    input_dir,
                } => {
                    core.mongodb_restore(
                        events,
                        op,
                        host,
                        db,
                        collections,
                        input_dir,
                        false,
                        drop_first,
                    )
                    .await
                }
                JobKind::RestoreArchive {
                    includes,
                    input_path,
                } => {
                    core.mongodb_restore_archive(events, op, host, includes, input_path, drop_first)
                        .await
                }
            }
        };
        let task = self.ctx.run(
            window,
            cx,
            future,
            move |this: &mut Self, result, window, cx| {
                if let Some(run) = this.run.as_mut() {
                    run.progress
                        .finish(result.as_ref().map(|_| ()).map_err(String::as_str));
                }
                match result {
                    Ok(()) => {
                        this.ctx
                            .toast(window, cx, job.success.clone(), ToastKind::Success);
                        if let Some(run) = this.run.as_mut() {
                            run.done.push(job.done_name());
                        }
                    }
                    Err(e) if is_cancel(&e) => {
                        this.ctx
                            .toast(window, cx, job.cancelled.clone(), ToastKind::Info);
                        return this.finish_run(window, cx);
                    }
                    Err(e) => {
                        this.ctx.toast(
                            window,
                            cx,
                            format!("{}: {e}", job.failed),
                            ToastKind::Error,
                        );
                    }
                }
                this.next_job(window, cx);
            },
        );
        if let Some(run) = self.run.as_mut() {
            run._task = Some(task);
        }
        cx.notify();
    }

    fn finish_run(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(run) = self.run.take() else {
            return;
        };
        let refresh = run.refresh_after;
        // One notice for the whole run, not one per database.
        let (title, verb) = if refresh {
            ("Restore complete", "Restored")
        } else {
            ("Backup complete", "Backed up")
        };
        if let Some(body) = completed_summary(verb, &run.done) {
            self.ctx.notify_if_unfocused(window, cx, title, body);
        }
        cx.notify();
        // Show what was just restored without reopening the tab.
        if refresh {
            self.load_databases(window, cx);
        }
    }

    fn cancel_run(&mut self, cx: &mut Context<Self>) {
        let Some(run) = self.run.as_mut() else {
            return;
        };
        if run.aborting {
            return;
        }
        run.aborting = true;
        run.progress.cancelling();
        self.ctx.cancel_op(&run.progress.op_id);
        cx.notify();
    }

    // ---- drawing -------------------------------------------------------

    fn render_header(&self, p: &Palette, cx: &mut Context<Self>) -> impl IntoElement {
        let can_backup = self.can_backup();
        let busy = self.busy();
        let selected = self.selection.count();
        let rule = || div().w(px(1.)).h(px(16.)).mx_1().bg(p.line);
        PanelHeader::new(self.name(cx))
            .subtitle(self.ctx.display_uri(cx))
            .subtitle_mono()
            .action(
                IconButton::new("mongo-refresh", IconName::RefreshCw, "Refresh databases")
                    .pending(self.loading)
                    .on_click(cx.listener(|this, _, window, cx| this.load_databases(window, cx))),
            )
            .action(rule())
            .action(
                Button::new("mongo-backup")
                    .xsmall()
                    .primary()
                    .icon(IconName::Download)
                    .label("Backup")
                    .tooltip(backup_tooltip(selected))
                    .disabled(!can_backup)
                    .on_click(cx.listener(|this, _, window, cx| this.backup_folder(window, cx))),
            )
            .action(
                Button::new("mongo-archive")
                    .xsmall()
                    .ghost()
                    .label("Archive")
                    .tooltip("Back up to a single archive file")
                    .disabled(!can_backup)
                    .on_click(cx.listener(|this, _, window, cx| this.backup_archive(window, cx))),
            )
            .action(rule())
            .action(
                Button::new("mongo-restore")
                    .xsmall()
                    .outline()
                    .icon(IconName::Upload)
                    .label("Restore")
                    .tooltip("Restore from a backup folder")
                    .disabled(busy)
                    .on_click(cx.listener(|this, _, window, cx| this.restore_folder(window, cx))),
            )
            .action(
                Button::new("mongo-restore-file")
                    .xsmall()
                    .ghost()
                    .label("From file")
                    .tooltip("Restore from an archive file")
                    .disabled(busy)
                    .on_click(cx.listener(|this, _, window, cx| this.restore_file(window, cx))),
            )
    }

    fn render_progress(&self, p: &Palette, cx: &mut Context<Self>) -> Option<AnyElement> {
        let run = self.run.as_ref()?;
        let view = cx.entity().downgrade();
        Some(
            div()
                .flex_none()
                .bg(p.surface)
                .border_b_1()
                .border_color(p.line)
                .child(op_progress_row(
                    "mongo-op",
                    &run.progress,
                    p,
                    move |_, cx| {
                        let _ = view.update(cx, |this, cx| this.cancel_run(cx));
                    },
                ))
                .into_any_element(),
        )
    }

    fn render_tree_head(&self, p: &Palette, cx: &mut Context<Self>) -> impl IntoElement {
        let selected = self.selection.count();
        h_flex()
            .flex_none()
            .justify_between()
            .px_3()
            .py_1p5()
            .border_b_1()
            .border_color(p.line)
            .child(
                text_2xs(div())
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(p.ink_2)
                    .child("DATABASES"),
            )
            .when(selected > 0, |d| {
                d.child(
                    h_flex()
                        .gap_1()
                        .child(
                            text_2xs(div())
                                .text_color(p.accent)
                                .child(format!("{selected} selected")),
                        )
                        .child(
                            text_2xs(div())
                                .id("mongo-clear-selection")
                                .text_color(p.ink_2)
                                .underline()
                                .cursor_pointer()
                                .hover(|s| s.text_color(p.ink))
                                .child("clear")
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.selection.clear();
                                    this.select_when_loaded.clear();
                                    cx.notify();
                                })),
                        ),
                )
            })
    }

    fn render_tree(&self, p: &Palette, cx: &mut Context<Self>) -> AnyElement {
        if self.loading && self.databases.is_empty() {
            return EmptyState::loading("mongo-dbs-loading", "Loading databases\u{2026}")
                .compact()
                .into_any_element();
        }
        let view = cx.entity().downgrade();
        if let Some(e) = &self.list_error {
            return EmptyState::error(
                "mongo-dbs-error",
                "Could not list databases",
                "Retry",
                move |_, window, cx| {
                    let _ = view.update(cx, |this, cx| this.load_databases(window, cx));
                },
            )
            .hint(e.clone())
            .compact()
            .into_any_element();
        }
        if self.databases.is_empty() {
            return EmptyState::empty(
                "mongo-dbs-empty",
                "No databases",
                "Refresh",
                move |_, window, cx| {
                    let _ = view.update(cx, |this, cx| this.load_databases(window, cx));
                },
            )
            .icon(Icon::new(IconName::Database))
            .hint("This connection has no databases to browse")
            .compact()
            .into_any_element();
        }
        v_flex()
            .py_1()
            .children(self.databases.iter().map(|db| self.render_db(db, p, cx)))
            .into_any_element()
    }

    fn render_db(&self, db: &DbEntry, p: &Palette, cx: &mut Context<Self>) -> AnyElement {
        let open = self.expanded.contains(&db.name);
        let state = self.selection.db_state(db);
        let name = db.name.clone();
        let (toggle_name, check_name) = (name.clone(), name.clone());
        let row = h_flex()
            .id(SharedString::from(format!("mongo-db-{name}")))
            .gap_1p5()
            .px_2()
            .py_1()
            .text_xs()
            .text_color(p.ink)
            .cursor_pointer()
            .hover(|s| s.bg(p.raised))
            .child(
                Icon::new(if open {
                    IconName::ChevronDown
                } else {
                    IconName::ChevronRight
                })
                .xsmall()
                .flex_none()
                .text_color(p.ink_2),
            )
            .child(tri_check(
                SharedString::from(format!("mongo-dbcheck-{name}")),
                state,
                match state {
                    CheckState::All => "Deselect every collection",
                    _ => "Select every collection",
                },
                p,
                cx.listener(move |this, _, window, cx| {
                    cx.stop_propagation();
                    this.toggle_db_selection(&check_name, window, cx);
                }),
            ))
            .child(
                Icon::new(IconName::Database)
                    .xsmall()
                    .flex_none()
                    .text_color(p.accent),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .child(SharedString::from(name.clone())),
            )
            .when(db.loading, |d| {
                d.child(text_2xs(div()).text_color(p.ink_3).child("\u{2026}"))
            })
            // An unexpanded database has no count to report; "0 cols"
            // would claim it is empty.
            .when(!db.collections.is_empty(), |d| {
                d.child(
                    text_2xs(div())
                        .flex_none()
                        .text_color(p.ink_3)
                        .child(format!("{} cols", db.collections.len())),
                )
            })
            .on_click(
                cx.listener(move |this, _, window, cx| this.toggle_db(&toggle_name, window, cx)),
            );

        let mut block = v_flex()
            .border_b_1()
            .border_color(p.line.opacity(0.3))
            .child(row);
        if open {
            let mut list = v_flex().pl_6().pr_2().py_1().gap_0p5();
            if db.loading && !db.loaded {
                list = list.child(
                    text_2xs(div())
                        .px_1()
                        .text_color(p.ink_3)
                        .child("Loading\u{2026}"),
                );
            } else if db.collections.is_empty() {
                list = list.child(
                    text_2xs(div())
                        .px_1()
                        .text_color(p.ink_3)
                        .child("No collections"),
                );
            }
            for coll in &db.collections {
                list = list.child(self.render_collection(&db.name, coll, p, cx));
            }
            block = block.child(list);
        }
        block.into_any_element()
    }

    fn render_collection(
        &self,
        db: &str,
        coll: &str,
        p: &Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let active = self
            .viewing
            .as_ref()
            .is_some_and(|(d, c)| d == db && c == coll);
        let selected = self.selection.is_selected(db, coll);
        let (d1, c1, d2, c2) = (
            db.to_string(),
            coll.to_string(),
            db.to_string(),
            coll.to_string(),
        );
        let sel_bg = p.selected;
        let raised = p.raised;
        h_flex()
            .id(SharedString::from(format!("mongo-coll-{db}\u{0}{coll}")))
            .gap_1p5()
            .px_1()
            .py_0p5()
            .rounded_sm()
            .text_xs()
            .text_color(p.ink)
            .cursor_pointer()
            .when(active, |d| d.bg(sel_bg))
            .hover(move |s| s.bg(if active { sel_bg } else { raised }))
            .tooltip(|window, cx| Tooltip::new("Click to view documents").build(window, cx))
            .child(
                Checkbox::new(SharedString::from(format!("mongo-check-{db}\u{0}{coll}")))
                    .checked(selected)
                    .xsmall()
                    .on_click(cx.listener(move |this, _, _, cx| {
                        cx.stop_propagation();
                        this.selection.toggle(&d1, &c1);
                        cx.notify();
                    })),
            )
            .child(
                Icon::new(IconName::Table)
                    .xsmall()
                    .flex_none()
                    .text_color(p.ink_3),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .when(active, |d| d.font_weight(FontWeight::MEDIUM))
                    .child(SharedString::from(coll.to_string())),
            )
            .on_click(
                cx.listener(move |this, _, window, cx| this.open_collection(&d2, &c2, window, cx)),
            )
            .into_any_element()
    }
}

impl Render for MongoTab {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = *palette(cx);
        let documents: AnyElement = match &self.documents {
            Some(view) => view.clone().into_any_element(),
            None => div()
                .size_full()
                .flex()
                .items_center()
                .justify_center()
                .child(
                    EmptyState::steps(
                        "mongo-pick-collection",
                        "Select a collection to view its documents",
                        text_2xs(div())
                            .text_color(p.ink_3)
                            .child("Tick collections to include them in a backup"),
                    )
                    .icon(Icon::new(IconName::FileSearch)),
                )
                .into_any_element(),
        };
        v_flex()
            .size_full()
            .bg(p.canvas)
            .child(self.render_header(&p, cx))
            .children(self.render_progress(&p, cx))
            .child(
                h_flex()
                    .flex_1()
                    .min_h_0()
                    .items_start()
                    .child(
                        v_flex()
                            .w(px(TREE_W))
                            .h_full()
                            .flex_none()
                            .border_r_1()
                            .border_color(p.line)
                            .child(self.render_tree_head(&p, cx))
                            .child(
                                div()
                                    .id("mongo-tree")
                                    .flex_1()
                                    .min_h_0()
                                    .overflow_y_scroll()
                                    .child(self.render_tree(&p, cx)),
                            ),
                    )
                    .child(div().flex_1().min_w_0().h_full().child(documents)),
            )
    }
}
