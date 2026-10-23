//! The SFTP panel. Port of `SftpPanel.vue` with `useSftpTransfers.js`, and of
//! `FloatingEditor.vue` (`editor.rs`) and `FilePreviewDialog.vue`
//! (`preview.rs`). Per tab: built once the tab's SFTP session is up, with
//! `ctx.sftp_session_id` set. See `../README.md`.
//!
//! The rules (sorting, selection, paths, the column preference, wording) are
//! in `model.rs` and `transfers.rs`, which are tested; this file lays them
//! out and runs the core calls.

mod editor;
mod model;
mod preview;
mod transfers;

use std::collections::VecDeque;
use std::future::Future;
use std::ops::Range;
use std::path::PathBuf;
use std::time::Instant;

use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::menu::{ContextMenuExt as _, DropdownMenu as _, PopupMenu, PopupMenuItem};
use gpui_kit::component::spinner::Spinner;
use gpui_kit::component::{ActiveTheme as _, Icon, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    Anchor, AnyElement, AnyView, App, AppContext as _, ClickEvent, ClipboardItem, Context, Entity,
    ExternalPaths, FontWeight, InteractiveElement, IntoElement, ParentElement, Render,
    SharedString, StatefulInteractiveElement as _, Styled, Subscription, Task, WeakEntity, Window,
    div, px, uniform_list,
};
use termdrop_core::CoreEvent;
use termdrop_core::sftp::SftpFile;

use self::editor::FloatingEditor;
use self::model::{
    ClickMods, Column, ColumnsPref, Selection, Sort, SortKey, breadcrumbs, bulk_delete_prompt,
    bulk_delete_result, bulk_download_result, child_path, delete_prompt, empty_hint,
    filter_indexes, local_file_name, renamed_path, sort_files, up_dir,
};
use self::transfers::{Finished, LINGER, Transfers};
use super::common::{
    format_bytes, format_file_date, format_permissions, format_speed, list_head, list_row, text_2xs,
};
use super::{EventKey, PanelContext, PanelSubscription};
use crate::ui::dialogs::{ConfirmOptions, PromptOptions, confirm, prompt};
use crate::ui::empty_state::EmptyState;
use crate::ui::file_dialog::pick_file;
use crate::ui::icon_button::IconButton;
use crate::ui::theme::{Palette, palette};
use crate::ui::toast::{ToastKind, toast};

const ROW_H: f32 = 22.;

/// The panel's view.
pub fn build(ctx: PanelContext, window: &mut Window, cx: &mut App) -> AnyView {
    cx.new(|cx| SftpPanel::new(ctx, window, cx)).into()
}

/// Run a core future on the runtime and hand its result to `done` on the
/// UI thread, if the view is still there. A panicked job reads as an error.
pub(super) fn run<V: 'static, T: Send + 'static>(
    ctx: &PanelContext,
    job: impl Future<Output = Result<T, String>> + Send + 'static,
    window: &mut Window,
    cx: &mut Context<V>,
    done: impl FnOnce(&mut V, Result<T, String>, &mut Window, &mut Context<V>) + 'static,
) -> Task<()> {
    let handle = ctx.spawn(job);
    cx.spawn_in(window, async move |this, cx| {
        let result = handle.await.unwrap_or_else(|e| Err(e.to_string()));
        let _ = cx.update(|window, cx| this.update(cx, |v, cx| done(v, result, window, cx)));
    })
}

/// One upload waiting its turn: (name, local path, remote path).
type Upload = (String, String, String);

pub struct SftpPanel {
    ctx: PanelContext,
    id: String,
    path: String,
    /// The listing, kept sorted by `sort`.
    files: Vec<SftpFile>,
    loading: bool,
    list_error: Option<String>,
    sort: Sort,
    columns: ColumnsPref,
    filter: Entity<InputState>,
    query: String,
    selection: Selection,
    transfers: Transfers,
    editor: Entity<FloatingEditor>,
    /// The listing in flight; replacing it drops the stale one's result.
    list_task: Option<Task<()>>,
    /// Uploads run one at a time, as the Vue loop awaited each.
    upload_queue: VecDeque<Upload>,
    uploading: bool,
    was_shown: bool,
    _progress: PanelSubscription,
    _subscriptions: Vec<Subscription>,
}

impl SftpPanel {
    fn new(ctx: PanelContext, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let id = ctx.sftp_session_id.clone().unwrap_or_default();
        let filter = cx.new(|cx| InputState::new(window, cx).placeholder("Filter files..."));
        let editor = cx.new(|cx| FloatingEditor::new(ctx.clone(), window, cx));
        let progress = ctx.subscribe(
            EventKey::SftpTransfer(id.clone()),
            cx.entity().downgrade(),
            |this: &mut Self, event, cx| {
                if let CoreEvent::SftpProgress {
                    file,
                    bytes_transferred,
                    total_bytes,
                    ..
                } = event
                {
                    let finished = this.transfers.progress(
                        file,
                        *bytes_transferred,
                        *total_bytes,
                        Instant::now(),
                    );
                    this.linger(finished, cx);
                    cx.notify();
                }
            },
        );
        let subscriptions = vec![
            cx.subscribe(&filter, |this, input, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    this.query = input.read(cx).value().to_string();
                    cx.notify();
                }
            }),
            // The column choice is one setting for every SFTP panel; another
            // panel may have changed it while this one was hidden.
            cx.observe(&ctx.model, |this, _, cx| {
                let shown = this.ctx.is_shown(cx);
                if shown && !this.was_shown {
                    this.columns = read_columns(&this.ctx, cx);
                    cx.notify();
                }
                this.was_shown = shown;
            }),
            cx.observe(&editor, |_, _, cx| cx.notify()),
        ];
        let columns = read_columns(&ctx, cx);
        let mut this = Self {
            ctx,
            id,
            path: "/".into(),
            files: Vec::new(),
            loading: true,
            list_error: None,
            sort: Sort::default(),
            columns,
            filter,
            query: String::new(),
            selection: Selection::default(),
            transfers: Transfers::default(),
            editor,
            list_task: None,
            upload_queue: VecDeque::new(),
            uploading: false,
            was_shown: true,
            _progress: progress,
            _subscriptions: subscriptions,
        };
        this.start(window, cx);
        this
    }

    /// Resolve the home directory, then list it. `/` when that fails, as the
    /// Vue panel fell back. `resolveHomeDir` + `loadFiles`.
    fn start(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let core = self.ctx.core().clone();
        let id = self.id.clone();
        self.list_task = Some(run(
            &self.ctx,
            async move { core.sftp_realpath(&id, ".".into()).await },
            window,
            cx,
            |this: &mut Self, home, window, cx| {
                this.path = match home {
                    Ok(home) if !home.is_empty() => home,
                    Ok(_) => this.path.clone(),
                    Err(e) => {
                        tracing::warn!("Failed to resolve home dir, falling back to /: {e}");
                        "/".into()
                    }
                };
                this.load(window, cx, |_, _, _, _| {});
            },
        ));
    }

    /// List the current directory; `after(ok)` once it is in. `loadFiles`.
    fn load(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
        after: impl FnOnce(&mut Self, bool, &mut Window, &mut Context<Self>) + 'static,
    ) {
        self.loading = true;
        let core = self.ctx.core().clone();
        let id = self.id.clone();
        let path = self.path.clone();
        self.list_task = Some(run(
            &self.ctx,
            async move { core.sftp_list(&id, path).await },
            window,
            cx,
            move |this: &mut Self, result, window, cx| {
                this.loading = false;
                let ok = match result {
                    Ok(mut files) => {
                        sort_files(&mut files, this.sort);
                        this.files = files;
                        this.list_error = None;
                        true
                    }
                    Err(e) => {
                        // Swallowing this made a dead session look like a
                        // panel that had stopped responding.
                        tracing::error!("sftp_list failed: {e}");
                        this.list_error = Some(e);
                        false
                    }
                };
                after(this, ok, window, cx);
                cx.notify();
            },
        ));
        cx.notify();
    }

    fn reload(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.load(window, cx, |_, _, _, _| {});
    }

    /// Move to `path`, going back if the listing fails so the path bar never
    /// names a directory other than the one listed. `navigateTo`.
    fn navigate(&mut self, path: String, window: &mut Window, cx: &mut Context<Self>) {
        let previous = std::mem::replace(&mut self.path, path);
        self.selection.clear();
        self.query.clear();
        self.filter.update(cx, |s, cx| s.set_value("", window, cx));
        self.load(window, cx, move |this, ok, _, _| {
            if !ok {
                this.path = previous;
            }
        });
    }

    fn go_up(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(up) = up_dir(&self.path) {
            self.navigate(up, window, cx);
        }
    }

    fn set_sort(&mut self, key: SortKey, cx: &mut Context<Self>) {
        self.sort = self.sort.toggle(key);
        sort_files(&mut self.files, self.sort);
        cx.notify();
    }

    fn toggle_column(&mut self, column: Column, cx: &mut Context<Self>) {
        let on = !self.columns.columns.get(column);
        self.columns.columns.set(column, on);
        let json = self.columns.to_json();
        if let Err(e) = self
            .ctx
            .model
            .read(cx)
            .store()
            .set_setting(model::COLUMNS_KEY, &json)
        {
            tracing::warn!("saving sftp-columns failed: {e}");
        }
        cx.notify();
    }

    /// The visible rows: indexes into `files`, filtered.
    fn visible(&self) -> Vec<usize> {
        filter_indexes(&self.files, &self.query)
    }

    fn file(&self, path: &str) -> Option<&SftpFile> {
        self.files.iter().find(|f| f.path == path)
    }

    /// Schedule the removal of a finished transfer row.
    fn linger(&mut self, finished: Finished, cx: &mut Context<Self>) {
        let Finished::Remove(key) = finished else {
            return;
        };
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(LINGER).await;
            let _ = this.update(cx, |this, cx| {
                this.transfers.remove(&key);
                cx.notify();
            });
        })
        .detach();
    }

    // ---- clicks ------------------------------------------------------------

    fn click_row(
        &mut self,
        index: usize,
        e: &ClickEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let visible = self.visible();
        let Some(file) = visible.get(index).map(|&i| self.files[i].clone()) else {
            return;
        };
        if e.click_count() >= 2 {
            if file.is_dir {
                self.navigate(file.path, window, cx);
            } else {
                self.preview(file, window, cx);
            }
            return;
        }
        let m = e.modifiers();
        let mods = ClickMods {
            toggle: m.control || m.platform,
            range: m.shift,
        };
        self.select(index, mods, cx);
    }

    fn select(&mut self, index: usize, mods: ClickMods, cx: &mut Context<Self>) {
        let visible = self.visible();
        let paths: Vec<&str> = visible
            .iter()
            .map(|&i| self.files[i].path.as_str())
            .collect();
        self.selection.click(&paths, index, mods);
        cx.notify();
    }

    // ---- actions -----------------------------------------------------------

    fn on_upload(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let panel = cx.entity().downgrade();
        pick_file(window, cx, "Upload", None, move |path, window, cx| {
            let _ = panel.update(cx, |this, cx| {
                this.upload_paths(vec![path], false, window, cx)
            });
        });
    }

    /// Queue local files for upload into the current directory. A drop
    /// toasts each success; the Upload button leaves that to the transfer
    /// row, as the Vue panel did. `uploadDroppedFiles`.
    fn upload_paths(
        &mut self,
        paths: Vec<PathBuf>,
        toast_success: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        for local in paths {
            let local = local.to_string_lossy().to_string();
            let Some(name) = local_file_name(&local).map(str::to_string) else {
                continue;
            };
            let remote = child_path(&self.path, &name);
            self.upload_queue.push_back((name, local, remote));
        }
        self.next_upload(toast_success, window, cx);
    }

    fn next_upload(&mut self, toast_success: bool, window: &mut Window, cx: &mut Context<Self>) {
        if self.uploading {
            return;
        }
        let Some((name, local, remote)) = self.upload_queue.pop_front() else {
            return;
        };
        self.uploading = true;
        let core = self.ctx.core().clone();
        let events = self.ctx.events();
        let id = self.id.clone();
        let target = remote.clone();
        run(
            &self.ctx,
            async move { core.sftp_upload(events, &id, local, target).await },
            window,
            cx,
            move |this: &mut Self, result, window, cx| {
                this.uploading = false;
                match result {
                    Ok(()) if toast_success => {
                        toast(window, cx, format!("Uploaded {name}"), ToastKind::Success)
                    }
                    Ok(()) => {}
                    Err(e) => {
                        tracing::error!("Upload failed: {e}");
                        let f = this.transfers.fail(&remote);
                        this.linger(f, cx);
                        toast(
                            window,
                            cx,
                            format!("Upload failed for {name}: {e}"),
                            ToastKind::Error,
                        );
                    }
                }
                if this.upload_queue.is_empty() {
                    this.reload(window, cx);
                } else {
                    this.next_upload(toast_success, window, cx);
                }
            },
        )
        .detach();
    }

    fn download(&mut self, file: SftpFile, window: &mut Window, cx: &mut Context<Self>) {
        if file.is_dir {
            return;
        }
        let core = self.ctx.core().clone();
        let events = self.ctx.events();
        let id = self.id.clone();
        let remote = file.path.clone();
        run(
            &self.ctx,
            async move { core.sftp_download(events, &id, remote).await },
            window,
            cx,
            move |this: &mut Self, result, window, cx| match result {
                Ok(saved) => toast(
                    window,
                    cx,
                    format!("Downloaded to {saved}"),
                    ToastKind::Success,
                ),
                Err(e) => {
                    let f = this.transfers.fail(&file.path);
                    this.linger(f, cx);
                    toast(
                        window,
                        cx,
                        format!("Download failed: {e}"),
                        ToastKind::Error,
                    );
                }
            },
        )
        .detach();
    }

    /// Tar the folder on the host and fetch it. Needs the shell, which keeps
    /// the host's exec session open. `onDownloadDir`.
    fn download_dir(&mut self, file: SftpFile, window: &mut Window, cx: &mut Context<Self>) {
        if !file.is_dir {
            return;
        }
        let key = self
            .transfers
            .begin_folder(&file.path, &file.name, Instant::now());
        cx.notify();
        let core = self.ctx.core().clone();
        let id = self.id.clone();
        let remote = file.path.clone();
        run(
            &self.ctx,
            async move { core.sftp_download_dir(&id, remote).await },
            window,
            cx,
            move |this: &mut Self, result, window, cx| {
                let ok = result.is_ok();
                let f = this.transfers.finish_folder(&key, &file.name, ok);
                this.linger(f, cx);
                match result {
                    Ok(saved) => toast(
                        window,
                        cx,
                        format!("Downloaded folder to {saved}"),
                        ToastKind::Success,
                    ),
                    Err(e) => toast(
                        window,
                        cx,
                        format!("Download folder failed: {e}"),
                        ToastKind::Error,
                    ),
                }
                cx.notify();
            },
        )
        .detach();
    }

    fn delete(&mut self, file: SftpFile, window: &mut Window, cx: &mut Context<Self>) {
        let (title, message) = delete_prompt(&file.name, file.is_dir);
        let panel = cx.entity().downgrade();
        confirm(
            window,
            cx,
            ConfirmOptions::new(title, message)
                .confirm_label("Delete")
                .danger(),
            move |yes, window, cx| {
                if !yes {
                    return;
                }
                let file = file.clone();
                let _ = panel.update(cx, |this, cx| this.delete_now(file, window, cx));
            },
        );
    }

    fn delete_now(&mut self, file: SftpFile, window: &mut Window, cx: &mut Context<Self>) {
        let core = self.ctx.core().clone();
        let id = self.id.clone();
        let path = file.path.clone();
        let is_dir = file.is_dir;
        run(
            &self.ctx,
            async move {
                if is_dir {
                    core.sftp_rmdir(&id, path).await
                } else {
                    core.sftp_delete(&id, path).await
                }
            },
            window,
            cx,
            move |this: &mut Self, result, window, cx| match result {
                Ok(()) => {
                    this.reload(window, cx);
                    let msg = if is_dir {
                        format!("Deleted folder \"{}\"", file.name)
                    } else {
                        format!("Deleted \"{}\"", file.name)
                    };
                    toast(window, cx, msg, ToastKind::Success);
                }
                Err(e) => toast(window, cx, format!("Delete failed: {e}"), ToastKind::Error),
            },
        )
        .detach();
    }

    fn bulk_delete(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let targets: Vec<(String, bool)> = self
            .selection
            .paths()
            .iter()
            .filter_map(|p| self.file(p).map(|f| (f.path.clone(), f.is_dir)))
            .collect();
        let count = self.selection.len();
        let panel = cx.entity().downgrade();
        confirm(
            window,
            cx,
            ConfirmOptions::new("Delete Selected", bulk_delete_prompt(count))
                .confirm_label("Delete")
                .danger(),
            move |yes, window, cx| {
                if !yes {
                    return;
                }
                let targets = targets.clone();
                let _ = panel.update(cx, |this, cx| this.bulk_delete_now(targets, window, cx));
            },
        );
    }

    fn bulk_delete_now(
        &mut self,
        targets: Vec<(String, bool)>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let core = self.ctx.core().clone();
        let id = self.id.clone();
        run(
            &self.ctx,
            async move {
                let (mut deleted, mut failed) = (0, 0);
                for (path, is_dir) in targets {
                    let r = if is_dir {
                        core.sftp_rmdir(&id, path).await
                    } else {
                        core.sftp_delete(&id, path).await
                    };
                    match r {
                        Ok(()) => deleted += 1,
                        Err(e) => {
                            tracing::error!("Delete failed: {e}");
                            failed += 1;
                        }
                    }
                }
                Ok((deleted, failed))
            },
            window,
            cx,
            |this: &mut Self, result, window, cx| {
                let (deleted, failed) = result.unwrap_or((0, 0));
                this.selection.clear();
                this.reload(window, cx);
                let (msg, warn) = bulk_delete_result(deleted, failed);
                let kind = if warn {
                    ToastKind::Warning
                } else {
                    ToastKind::Success
                };
                toast(window, cx, msg, kind);
            },
        )
        .detach();
    }

    fn bulk_download(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let targets: Vec<String> = self
            .selection
            .paths()
            .iter()
            .filter(|p| self.file(p).is_some_and(|f| !f.is_dir))
            .cloned()
            .collect();
        let core = self.ctx.core().clone();
        let events = self.ctx.events();
        let id = self.id.clone();
        run(
            &self.ctx,
            async move {
                let mut completed = 0;
                let mut failed = Vec::new();
                for path in targets {
                    match core.sftp_download(events.clone(), &id, path.clone()).await {
                        Ok(_) => completed += 1,
                        Err(e) => {
                            tracing::error!("Download failed: {e}");
                            failed.push(path);
                        }
                    }
                }
                Ok((completed, failed))
            },
            window,
            cx,
            |this: &mut Self, result, window, cx| {
                let (completed, failed) = result.unwrap_or_default();
                for path in &failed {
                    let f = this.transfers.fail(path);
                    this.linger(f, cx);
                }
                this.selection.clear();
                let (msg, warn) = bulk_download_result(completed, failed.len());
                let kind = if warn {
                    ToastKind::Warning
                } else {
                    ToastKind::Success
                };
                toast(window, cx, msg, kind);
                cx.notify();
            },
        )
        .detach();
    }

    fn mkdir(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let panel = cx.entity().downgrade();
        prompt(
            window,
            cx,
            PromptOptions::new("New Folder")
                .message("Enter a name for the new folder:")
                .placeholder("folder-name"),
            move |name, window, cx| {
                let Some(name) = name.map(|n| n.trim().to_string()).filter(|n| !n.is_empty())
                else {
                    return;
                };
                let _ = panel.update(cx, |this, cx| this.mkdir_now(name, window, cx));
            },
        );
    }

    fn mkdir_now(&mut self, name: String, window: &mut Window, cx: &mut Context<Self>) {
        let core = self.ctx.core().clone();
        let id = self.id.clone();
        let path = child_path(&self.path, &name);
        run(
            &self.ctx,
            async move { core.sftp_mkdir(&id, path).await },
            window,
            cx,
            move |this: &mut Self, result, window, cx| match result {
                Ok(()) => {
                    this.reload(window, cx);
                    toast(
                        window,
                        cx,
                        format!("Created folder \"{name}\""),
                        ToastKind::Success,
                    );
                }
                Err(e) => toast(
                    window,
                    cx,
                    format!("Failed to create folder: {e}"),
                    ToastKind::Error,
                ),
            },
        )
        .detach();
    }

    fn rename(&mut self, file: SftpFile, window: &mut Window, cx: &mut Context<Self>) {
        let panel = cx.entity().downgrade();
        let old_name = file.name.clone();
        prompt(
            window,
            cx,
            PromptOptions::new("Rename")
                .message(format!("Rename \"{}\" to:", file.name))
                .placeholder("new-name")
                .initial(file.name.clone()),
            move |name, window, cx| {
                let Some(name) = name
                    .map(|n| n.trim().to_string())
                    .filter(|n| !n.is_empty() && *n != old_name)
                else {
                    return;
                };
                let path = file.path.clone();
                let _ = panel.update(cx, |this, cx| this.rename_now(path, name, window, cx));
            },
        );
    }

    fn rename_now(
        &mut self,
        path: String,
        name: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let core = self.ctx.core().clone();
        let id = self.id.clone();
        let new_path = renamed_path(&path, &name);
        run(
            &self.ctx,
            async move { core.sftp_rename(&id, path, new_path).await },
            window,
            cx,
            move |this: &mut Self, result, window, cx| match result {
                Ok(()) => {
                    this.reload(window, cx);
                    toast(
                        window,
                        cx,
                        format!("Renamed to \"{name}\""),
                        ToastKind::Success,
                    );
                }
                Err(e) => toast(window, cx, format!("Rename failed: {e}"), ToastKind::Error),
            },
        )
        .detach();
    }

    fn copy_path(&mut self, file: &SftpFile, window: &mut Window, cx: &mut Context<Self>) {
        cx.write_to_clipboard(ClipboardItem::new_string(file.path.clone()));
        toast(window, cx, "Path copied to clipboard", ToastKind::Success);
    }

    /// Open the preview, closing the editor first (it asks if it has unsaved
    /// changes, and a "keep editing" answer opens nothing). `onPreviewFile`.
    fn preview(&mut self, file: SftpFile, window: &mut Window, cx: &mut Context<Self>) {
        if file.is_dir {
            return;
        }
        let panel = cx.entity().downgrade();
        let open = move |window: &mut Window, cx: &mut App| {
            let Some(this) = panel.upgrade() else {
                return;
            };
            let ctx = this.read(cx).ctx.clone();
            let (p_edit, p_dl) = (panel.clone(), panel.clone());
            let (f_edit, f_dl) = (file.clone(), file.clone());
            preview::open_preview(
                &ctx,
                file.name.clone(),
                file.path.clone(),
                window,
                cx,
                move |window, cx| {
                    let f = f_edit.clone();
                    let _ = p_edit.update(cx, |this, cx| this.edit(f, window, cx));
                },
                move |window, cx| {
                    let f = f_dl.clone();
                    let _ = p_dl.update(cx, |this, cx| this.download(f, window, cx));
                },
            );
        };
        if self.editor.read(cx).is_open() {
            self.editor.update(cx, |e, cx| {
                e.request_hide(window, cx, move |closed, window, cx| {
                    if closed {
                        open(window, cx);
                    }
                })
            });
        } else {
            open(window, cx);
        }
    }

    fn edit(&mut self, file: SftpFile, window: &mut Window, cx: &mut Context<Self>) {
        if file.is_dir {
            return;
        }
        self.editor
            .update(cx, |e, cx| e.open(file.name, file.path, window, cx));
    }

    // ---- menus -------------------------------------------------------------

    /// The row menu: the selection's bulk actions when the row is part of a
    /// multiple selection, otherwise the row's own. `showContextMenu`.
    fn row_menu(
        panel: &WeakEntity<Self>,
        file: &SftpFile,
        menu: PopupMenu,
        cx: &mut App,
    ) -> PopupMenu {
        let Some(multi) = panel
            .update(cx, |this, cx| {
                let multi = this.selection.context(&file.path);
                cx.notify();
                (multi, this.selection.len())
            })
            .ok()
        else {
            return menu;
        };
        let p = *palette(cx);
        let act = |f: fn(&mut Self, SftpFile, &mut Window, &mut Context<Self>)| {
            let panel = panel.clone();
            let file = file.clone();
            move |_: &ClickEvent, window: &mut Window, cx: &mut App| {
                let file = file.clone();
                let _ = panel.update(cx, |this, cx| f(this, file, window, cx));
            }
        };
        let (is_multi, count) = multi;
        if is_multi {
            let (d, x, c) = (panel.clone(), panel.clone(), panel.clone());
            return menu
                .item(
                    PopupMenuItem::new(format!("Download Selected ({count})"))
                        .icon(IconName::Download)
                        .on_click(move |_, window, cx| {
                            let _ = d.update(cx, |this, cx| this.bulk_download(window, cx));
                        }),
                )
                .item(
                    PopupMenuItem::new(format!("Delete Selected ({count})"))
                        .icon(Icon::new(IconName::Trash).text_color(p.bad))
                        .on_click(move |_, window, cx| {
                            let _ = x.update(cx, |this, cx| this.bulk_delete(window, cx));
                        }),
                )
                .item(
                    PopupMenuItem::new("Clear Selection").on_click(move |_, _, cx| {
                        let _ = c.update(cx, |this, cx| {
                            this.selection.clear();
                            cx.notify();
                        });
                    }),
                );
        }
        let mut menu = menu;
        if file.is_dir {
            menu = menu.item(
                PopupMenuItem::new("Download Folder")
                    .icon(IconName::Download)
                    .on_click(act(Self::download_dir)),
            );
        } else {
            menu = menu
                .item(
                    PopupMenuItem::new("Preview")
                        .icon(IconName::Eye)
                        .on_click(act(Self::preview)),
                )
                .item(
                    PopupMenuItem::new("Edit")
                        .icon(IconName::Pencil)
                        .on_click(act(Self::edit)),
                )
                .item(
                    PopupMenuItem::new("Download")
                        .icon(IconName::Download)
                        .on_click(act(Self::download)),
                );
        }
        menu.item(
            PopupMenuItem::new("Copy Path")
                .icon(IconName::Copy)
                .on_click(act(|this, f, w, cx| this.copy_path(&f, w, cx))),
        )
        .item(
            PopupMenuItem::new("Rename")
                .icon(IconName::PencilLine)
                .on_click(act(Self::rename)),
        )
        .item(
            PopupMenuItem::new("Delete")
                .icon(Icon::new(IconName::Trash).text_color(p.bad))
                .on_click(act(Self::delete)),
        )
    }

    // ---- rendering ---------------------------------------------------------

    fn render_path_bar(&self, p: &Palette, cx: &mut Context<Self>) -> impl IntoElement {
        let crumbs = breadcrumbs(&self.path);
        let last = crumbs.len() - 1;
        let mut bar = h_flex()
            .id("sftp-path")
            .flex_none()
            .px_2()
            .py_1()
            .gap_0p5()
            .border_b_1()
            .border_color(p.line)
            .text_xs()
            .overflow_x_scroll();
        for (i, crumb) in crumbs.into_iter().enumerate() {
            let path = crumb.path.clone();
            let item: AnyElement =
                if i == 0 {
                    IconButton::new("sftp-root", IconName::House, "Go to root")
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.navigate("/".into(), window, cx)
                        }))
                        .into_any_element()
                } else if i == last {
                    div()
                        .px_1()
                        .rounded_sm()
                        .bg(p.input.opacity(0.5))
                        .text_color(p.ink)
                        .font_weight(FontWeight::MEDIUM)
                        .max_w(px(120.))
                        .truncate()
                        .child(crumb.name)
                        .into_any_element()
                } else {
                    div()
                        .id(SharedString::from(format!("sftp-crumb-{i}")))
                        .px_1()
                        .rounded_sm()
                        .text_color(p.ink_2)
                        .max_w(px(120.))
                        .truncate()
                        .cursor_pointer()
                        .hover(|s| s.text_color(p.ink).bg(p.input.opacity(0.3)))
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.navigate(path.clone(), window, cx)
                        }))
                        .child(crumb.name)
                        .into_any_element()
                };
            bar = bar.child(div().flex_none().child(item));
            if i < last {
                bar = bar.child(
                    Icon::new(IconName::ChevronRight)
                        .xsmall()
                        .text_color(p.ink_3)
                        .flex_none(),
                );
            }
        }
        bar
    }

    fn render_toolbar(&self, p: &Palette, cx: &mut Context<Self>) -> impl IntoElement {
        let panel = cx.entity().downgrade();
        let columns = self.columns.columns;
        h_flex()
            .flex_none()
            .px_2()
            .py_1()
            .gap_1p5()
            .border_b_1()
            .border_color(p.line)
            .child(
                IconButton::new("sftp-up", IconName::ArrowUp, "Go up a directory")
                    .on_click(cx.listener(|this, _, window, cx| this.go_up(window, cx))),
            )
            .child(
                Button::new("sftp-upload")
                    .primary()
                    .xsmall()
                    .label("Upload")
                    .on_click(cx.listener(|this, _, window, cx| this.on_upload(window, cx))),
            )
            .child(
                Button::new("sftp-mkdir")
                    .xsmall()
                    .label("+ Folder")
                    .on_click(cx.listener(|this, _, window, cx| this.mkdir(window, cx))),
            )
            .child(
                IconButton::new(
                    "sftp-refresh",
                    IconName::RefreshCw,
                    "Refresh this directory",
                )
                .disabled(self.loading)
                .on_click(cx.listener(|this, _, window, cx| this.reload(window, cx))),
            )
            .child(div().flex_1())
            .child(
                Button::new("sftp-columns")
                    .ghost()
                    .small()
                    .icon(Icon::new(IconName::Columns3))
                    .tooltip("Choose columns")
                    .accessibility_label("Choose columns")
                    .dropdown_menu_with_anchor(Anchor::TopRight, move |mut menu, _, _| {
                        for column in Column::ALL {
                            let panel = panel.clone();
                            menu = menu.item(
                                PopupMenuItem::new(column.label())
                                    .checked(columns.get(column))
                                    .on_click(move |_, _, cx| {
                                        let _ = panel
                                            .update(cx, |this, cx| this.toggle_column(column, cx));
                                    }),
                            );
                        }
                        menu
                    }),
            )
    }

    fn render_head(&self, p: &Palette, cx: &mut Context<Self>) -> impl IntoElement {
        let cols = self.columns.columns;
        let sort = self.sort;
        let head = |id: &'static str, label: &'static str, key: SortKey| {
            div()
                .id(id)
                .cursor_pointer()
                .hover(|s| s.text_color(p.ink))
                .child(format!("{label} {}", sort.indicator(key)))
                .on_click(cx.listener(move |this, _, _, cx| this.set_sort(key, cx)))
        };
        list_head(h_flex().flex_none(), p)
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .child(head("sftp-sort-name", "NAME", SortKey::Name)),
            )
            .when(cols.size, |d| {
                d.child(h_flex().w_12().flex_none().justify_end().child(head(
                    "sftp-sort-size",
                    "SIZE",
                    SortKey::Size,
                )))
            })
            .when(cols.modified, |d| {
                d.child(
                    h_flex()
                        .w(px(56.))
                        .ml_1p5()
                        .flex_none()
                        .justify_end()
                        .child(head("sftp-sort-modified", "MODIFIED", SortKey::Modified)),
                )
            })
            .when(cols.perms, |d| {
                d.child(
                    h_flex()
                        .w_16()
                        .ml_1p5()
                        .flex_none()
                        .justify_end()
                        .child("PERMS"),
                )
            })
    }

    fn render_rows(
        &mut self,
        range: Range<usize>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        let p = *palette(cx);
        let cols = self.columns.columns;
        let visible = self.visible();
        let panel = cx.entity().downgrade();
        let mono = cx.theme().mono_font_family.clone();
        range
            .filter_map(|index| {
                let file = self.files.get(*visible.get(index)?)?.clone();
                let selected = self.selection.contains(&file.path);
                let tone = if file.is_dir { p.accent } else { p.ink };
                let muted = |d: gpui_kit::Div| d.flex_none().text_xs().text_color(p.ink_3);
                let menu_panel = panel.clone();
                let menu_file = file.clone();
                let row = h_flex()
                    .id(SharedString::from(format!("sftp-row-{}", file.path)))
                    .h(px(ROW_H))
                    .px_2()
                    .text_xs()
                    .text_color(tone);
                let row = list_row(row, &p, selected, true)
                    .child(
                        div().mr_1p5().flex_none().child(
                            Checkbox::new(SharedString::from(format!("sftp-check-{}", file.path)))
                                .checked(selected)
                                .xsmall()
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    cx.stop_propagation();
                                    this.select(
                                        index,
                                        ClickMods {
                                            toggle: true,
                                            range: false,
                                        },
                                        cx,
                                    );
                                })),
                        ),
                    )
                    .child(
                        Icon::new(if file.is_dir {
                            IconName::Folder
                        } else {
                            IconName::FileText
                        })
                        .xsmall()
                        .mr_1p5()
                        .flex_none()
                        .text_color(if file.is_dir {
                            p.accent
                        } else {
                            p.ink_3
                        }),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .child(SharedString::from(file.name.clone())),
                    )
                    .when(cols.size, |d| {
                        d.child(
                            muted(div())
                                .w_12()
                                .flex()
                                .justify_end()
                                .child(if file.is_dir {
                                    "-".to_string()
                                } else {
                                    format_bytes(file.size)
                                }),
                        )
                    })
                    .when(cols.modified, |d| {
                        d.child(
                            muted(div())
                                .w(px(56.))
                                .ml_1p5()
                                .flex()
                                .justify_end()
                                .child(format_file_date(file.modified)),
                        )
                    })
                    .when(cols.perms, |d| {
                        d.child(
                            muted(div())
                                .w_16()
                                .ml_1p5()
                                .flex()
                                .justify_end()
                                .font_family(mono.clone())
                                .child(format_permissions(file.permissions, file.is_dir)),
                        )
                    })
                    .on_click(cx.listener(move |this, e: &ClickEvent, window, cx| {
                        this.click_row(index, e, window, cx)
                    }))
                    .context_menu(move |menu, _, cx| {
                        Self::row_menu(&menu_panel, &menu_file, menu, cx)
                    });
                Some(row.into_any_element())
            })
            .collect()
    }

    fn render_list(&self, p: &Palette, cx: &mut Context<Self>) -> AnyElement {
        if self.loading {
            return EmptyState::loading("sftp-loading", "Reading directory\u{2026}")
                .compact()
                .into_any_element();
        }
        if let Some(error) = &self.list_error {
            let panel = cx.entity().downgrade();
            return EmptyState::empty(
                "sftp-error",
                "Could not list this directory",
                "Retry",
                move |_, window, cx| {
                    let _ = panel.update(cx, |this, cx| this.reload(window, cx));
                },
            )
            .icon(Icon::new(IconName::TriangleAlert).text_color(p.bad))
            .hint(error.clone())
            .compact()
            .into_any_element();
        }
        let count = self.visible().len();
        if count == 0 {
            let panel = cx.entity().downgrade();
            return match empty_hint(self.files.len()) {
                None => EmptyState::empty(
                    "sftp-empty",
                    "Empty directory",
                    "Upload",
                    move |_, window, cx| {
                        let _ = panel.update(cx, |this, cx| this.on_upload(window, cx));
                    },
                )
                .icon(Icon::new(IconName::Folder))
                .compact()
                .into_any_element(),
                Some(hint) => EmptyState::filtered(
                    "sftp-filtered",
                    "No matching files",
                    "Clear filter",
                    move |_, window, cx| {
                        let _ = panel.update(cx, |this, cx| {
                            this.query.clear();
                            this.filter.update(cx, |s, cx| s.set_value("", window, cx));
                            cx.notify();
                        });
                    },
                )
                .icon(Icon::new(IconName::Folder))
                .hint(hint)
                .compact()
                .into_any_element(),
            };
        }
        uniform_list("sftp-files", count, cx.processor(Self::render_rows))
            .size_full()
            .into_any_element()
    }

    fn render_transfers(&self, p: &Palette) -> impl IntoElement {
        let rows = self.transfers.rows();
        v_flex()
            .flex_none()
            .border_t_1()
            .border_color(p.line)
            .bg(p.canvas)
            .child(
                text_2xs(div())
                    .px_2()
                    .py_1()
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(p.ink_3)
                    .border_b_1()
                    .border_color(p.line.opacity(0.5))
                    .child(format!("TRANSFERS ({})", rows.len())),
            )
            .child(
                v_flex()
                    .id("sftp-transfers")
                    .p_2()
                    .gap_2()
                    .max_h(px(128.))
                    .overflow_y_scroll()
                    .children(rows.iter().map(|t| {
                        let preparing = t.preparing();
                        let pct = t.percent();
                        let status: AnyElement = if !t.done && t.speed > 0.0 {
                            div()
                                .text_color(p.good)
                                .child(format_speed(t.speed))
                                .into_any_element()
                        } else if t.done && t.failed {
                            div().text_color(p.bad).child("Failed").into_any_element()
                        } else if t.done {
                            div().text_color(p.accent).child("Done").into_any_element()
                        } else {
                            div().into_any_element()
                        };
                        let fill = if t.failed {
                            p.bad
                        } else if t.done {
                            p.good
                        } else {
                            p.accent
                        };
                        v_flex()
                            .text_xs()
                            .child(
                                h_flex()
                                    .justify_between()
                                    .text_color(p.ink_2)
                                    .mb_0p5()
                                    .child(
                                        h_flex()
                                            .flex_1()
                                            .min_w_0()
                                            .mr_2()
                                            .gap_1()
                                            .when(t.folder, |d| {
                                                d.child(Icon::new(IconName::Folder).xsmall())
                                            })
                                            .child(div().truncate().child(t.label.clone())),
                                    )
                                    .child(
                                        div()
                                            .flex_none()
                                            .text_color(p.ink)
                                            .font_weight(FontWeight::MEDIUM)
                                            .child(if preparing {
                                                Spinner::new()
                                                    .xsmall()
                                                    .color(p.accent)
                                                    .into_any_element()
                                            } else {
                                                format!("{pct}%").into_any_element()
                                            }),
                                    ),
                            )
                            .child(
                                text_2xs(h_flex())
                                    .justify_between()
                                    .text_color(p.ink_3)
                                    .mb_1()
                                    .child(if preparing {
                                        "Preparing archive...".to_string()
                                    } else {
                                        format!(
                                            "{} / {}",
                                            format_bytes(t.bytes),
                                            format_bytes(t.total)
                                        )
                                    })
                                    .child(status),
                            )
                            .child(
                                div()
                                    .h(px(6.))
                                    .w_full()
                                    .rounded_sm()
                                    .bg(p.input)
                                    .overflow_hidden()
                                    .child(
                                        div()
                                            .h_full()
                                            .rounded_sm()
                                            .bg(if preparing { fill.opacity(0.5) } else { fill })
                                            .w(gpui_kit::relative(if preparing {
                                                1.0
                                            } else {
                                                f32::from(pct) / 100.0
                                            })),
                                    ),
                            )
                    })),
            )
    }
}

fn read_columns(ctx: &PanelContext, cx: &App) -> ColumnsPref {
    let raw = ctx
        .model
        .read(cx)
        .store()
        .get_setting(model::COLUMNS_KEY)
        .ok()
        .flatten();
    ColumnsPref::parse(raw.as_deref())
}

impl Render for SftpPanel {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = *palette(cx);
        let drop_tint = p.accent.opacity(0.08);
        let drop_line = p.accent;
        v_flex()
            .id("sftp-panel")
            .size_full()
            .bg(p.surface)
            .relative()
            .drag_over::<ExternalPaths>(move |s, _, _, _| {
                s.bg(drop_tint).border_1().border_color(drop_line)
            })
            .on_drop(cx.listener(|this, paths: &ExternalPaths, window, cx| {
                this.upload_paths(paths.paths().to_vec(), true, window, cx);
            }))
            .child(self.render_path_bar(&p, cx))
            .child(self.render_toolbar(&p, cx))
            .child(self.render_head(&p, cx))
            .child(
                div()
                    .flex_none()
                    .px_2()
                    .py_1()
                    .border_b_1()
                    .border_color(p.line)
                    .child(Input::new(&self.filter).xsmall()),
            )
            .child(div().flex_1().min_h_0().child(self.render_list(&p, cx)))
            .when(!self.transfers.is_empty(), |d| {
                d.child(self.render_transfers(&p))
            })
            .child(self.editor.clone())
    }
}
