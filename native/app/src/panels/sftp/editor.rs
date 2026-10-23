//! The floating text editor for a remote file. Port of `FloatingEditor.vue`:
//! a window over the app that can be dragged by its title and resized from
//! its corner, keeps its place and size across files, asks before dropping
//! unsaved changes, and refuses to overwrite a file that changed on the
//! server without asking (size and mtime from `sftp_stat_file`, taken at
//! load and after each save).
//!
//! It floats over the window (`PanelContext::float`): above the panels and
//! terminals, below dialogs and notifications, so its own "Unsaved Changes"
//! question appears over it. It draws nothing while its SFTP panel is not on
//! screen, so it hides with the panel and comes back with it, as the Vue
//! editor did inside its `KeepAlive`.

use std::cell::Cell;

use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Editor, EditorState, InputEvent};
use gpui_kit::component::{Disableable as _, Selectable as _, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    App, AppContext as _, Context, CursorStyle, DragMoveEvent, Empty, Entity, InteractiveElement,
    IntoElement, KeyDownEvent, MouseButton, MouseDownEvent, ParentElement, Pixels, Point, Render,
    SharedString, StatefulInteractiveElement as _, Styled, Subscription, Task, Window, anchored,
    div, point, px,
};
use termdrop_core::sftp::SftpStat;

use super::run;
use crate::panels::PanelContext;
use crate::panels::common::{display_name, text_2xs};
use crate::ui::dialogs::{ConfirmOptions, confirm};
use crate::ui::icon_button::IconButton;
use crate::ui::theme::palette;
use crate::ui::toast::{ToastKind, toast};

const MIN_W: f32 = 400.;
const MIN_H: f32 = 250.;

/// Drag payloads: GPUI tells drags apart by type.
#[derive(Clone)]
struct MoveEditor;
#[derive(Clone)]
struct ResizeEditor;

/// What follows the cursor during a drag: nothing; the editor itself moves.
struct Ghost;

impl Render for Ghost {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        Empty
    }
}

/// The file being edited.
struct Doc {
    name: String,
    path: String,
    /// The text as last loaded or saved: the buffer is dirty when it differs.
    original: String,
    loading: bool,
    saving: bool,
    dirty: bool,
    chars: usize,
    /// Size and mtime at the last load or save.
    stat: Option<SftpStat>,
}

pub struct FloatingEditor {
    ctx: PanelContext,
    sftp_id: String,
    doc: Option<Doc>,
    editor: Entity<EditorState>,
    word_wrap: bool,
    /// Window coordinates of the top-left corner; placed on first open.
    origin: Option<Point<Pixels>>,
    width: f32,
    height: f32,
    /// (mouse, origin) when a title drag started.
    move_from: Option<(Point<Pixels>, Point<Pixels>)>,
    /// (mouse, (w, h)) when a corner drag started.
    resize_from: Option<(Point<Pixels>, (f32, f32))>,
    task: Option<Task<()>>,
    _subscriptions: Vec<Subscription>,
}

impl FloatingEditor {
    pub fn new(ctx: PanelContext, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let sftp_id = ctx.sftp_session_id.clone().unwrap_or_default();
        let editor = cx.new(|cx| {
            EditorState::new(window, cx)
                .soft_wrap(false)
                .searchable(false)
        });
        let subscriptions = vec![
            cx.subscribe(&editor, |this, state, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change)
                    && let Some(doc) = this.doc.as_mut()
                    && !doc.loading
                {
                    let value = state.read(cx).value();
                    doc.dirty = value.as_ref() != doc.original;
                    doc.chars = value.chars().count();
                    cx.notify();
                }
            }),
        ];
        Self {
            ctx,
            sftp_id,
            doc: None,
            editor,
            word_wrap: false,
            origin: None,
            width: 560.,
            height: 400.,
            move_from: None,
            resize_from: None,
            task: None,
            _subscriptions: subscriptions,
        }
    }

    pub fn is_open(&self) -> bool {
        self.doc.is_some()
    }

    fn is_dirty(&self) -> bool {
        self.doc.as_ref().is_some_and(|d| d.dirty)
    }

    /// Ask before dropping unsaved changes; `then(true)` when the user said
    /// discard. `confirmDiscard`. Callers handle a clean buffer themselves,
    /// without asking: `then` runs outside this view's update, and calling it
    /// inside would update the view while it is already being updated.
    fn confirm_discard(
        &self,
        window: &mut Window,
        cx: &mut App,
        then: impl FnOnce(bool, &mut Window, &mut App) + 'static,
    ) {
        let Some(doc) = self.doc.as_ref().filter(|d| d.dirty) else {
            then(true, window, cx);
            return;
        };
        let then = Cell::new(Some(then));
        confirm(
            window,
            cx,
            ConfirmOptions::new(
                "Unsaved Changes",
                format!(
                    "You have unsaved changes in \"{}\". Discard them?",
                    display_name(&doc.name)
                ),
            )
            .confirm_label("Discard")
            .danger(),
            move |yes, window, cx| {
                if let Some(then) = then.take() {
                    then(yes, window, cx);
                }
            },
        );
    }

    /// Hide the editor, asking first if there are unsaved changes.
    /// `then(false)` when the user chose to keep editing, so a caller about
    /// to open something on top can stand down. `hide`.
    pub fn request_hide(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
        then: impl FnOnce(bool, &mut Window, &mut App) + 'static,
    ) {
        if !self.is_dirty() {
            self.close_now(cx);
            // After this update: `then` may read the panel that is updating
            // this view.
            window.defer(cx, move |window, cx| then(true, window, cx));
            return;
        }
        let this = cx.entity().downgrade();
        self.confirm_discard(window, cx, move |yes, window, cx| {
            if yes {
                let _ = this.update(cx, |e, cx| e.close_now(cx));
            }
            then(yes, window, cx);
        });
    }

    fn close_now(&mut self, cx: &mut Context<Self>) {
        self.doc = None;
        self.task = None;
        cx.notify();
    }

    /// Open `path` in the editor, replacing the current file once any
    /// unsaved changes are dealt with. `open`.
    pub fn open(
        &mut self,
        name: String,
        path: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.is_dirty() {
            self.load(name, path, window, cx);
            return;
        }
        let this = cx.entity().downgrade();
        self.confirm_discard(window, cx, move |yes, window, cx| {
            if yes {
                let _ = this.update(cx, |e, cx| e.load(name, path, window, cx));
            }
        });
    }

    fn load(&mut self, name: String, path: String, window: &mut Window, cx: &mut Context<Self>) {
        if self.origin.is_none() {
            let vp = window.viewport_size();
            self.origin = Some(point(
                (vp.width - px(580.)).max(px(0.)),
                (vp.height - px(420.)).max(px(0.)),
            ));
        }
        self.word_wrap = false;
        self.editor
            .update(cx, |s, cx| s.set_soft_wrap(false, window, cx));
        self.doc = Some(Doc {
            name,
            path: path.clone(),
            original: String::new(),
            loading: true,
            saving: false,
            dirty: false,
            chars: 0,
            stat: None,
        });
        let core = self.ctx.core().clone();
        let id = self.sftp_id.clone();
        self.task = Some(run(
            &self.ctx,
            async move {
                let text = core.sftp_read_file(&id, path.clone()).await?;
                // Not being able to stat is not a reason to refuse the file.
                let stat = core.sftp_stat_file(&id, path).await.ok();
                Ok((text, stat))
            },
            window,
            cx,
            |this: &mut Self, result, window, cx| match result {
                Ok((text, stat)) => {
                    if let Some(doc) = this.doc.as_mut() {
                        doc.original = text.clone();
                        doc.chars = text.chars().count();
                        doc.stat = stat;
                        doc.loading = false;
                        doc.dirty = false;
                    }
                    this.editor.update(cx, |s, cx| {
                        s.set_value(text, window, cx);
                        s.focus(window, cx);
                    });
                    cx.notify();
                }
                Err(e) => {
                    toast(
                        window,
                        cx,
                        format!("Failed to load file: {e}"),
                        ToastKind::Error,
                    );
                    this.close_now(cx);
                }
            },
        ));
        cx.notify();
    }

    /// Save, after checking the file did not change on the server since it
    /// was loaded or last saved. `onEditorSave`.
    fn save(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(doc) = self.doc.as_ref() else {
            return;
        };
        if !doc.dirty || doc.saving || doc.loading {
            return;
        }
        let core = self.ctx.core().clone();
        let id = self.sftp_id.clone();
        let path = doc.path.clone();
        self.task = Some(run(
            &self.ctx,
            async move { Ok(core.sftp_stat_file(&id, path).await.ok()) },
            window,
            cx,
            |this: &mut Self, current, window, cx| {
                let Some(doc) = this.doc.as_ref() else {
                    return;
                };
                let changed = match (&doc.stat, current.ok().flatten()) {
                    (Some(was), Some(now)) => was.modified != now.modified || was.size != now.size,
                    _ => false,
                };
                if !changed {
                    this.write(window, cx);
                    return;
                }
                let editor = cx.entity().downgrade();
                confirm(
                    window,
                    cx,
                    ConfirmOptions::new(
                        "File Changed on Server",
                        format!(
                            "\"{}\" was modified on the server after you opened it. \
                             Overwrite those changes?",
                            display_name(&doc.name)
                        ),
                    )
                    .confirm_label("Overwrite")
                    .danger(),
                    move |yes, window, cx| {
                        if yes {
                            let _ = editor.update(cx, |e, cx| e.write(window, cx));
                        }
                    },
                );
            },
        ));
    }

    fn write(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let content = self.editor.read(cx).value().to_string();
        let Some(doc) = self.doc.as_mut() else {
            return;
        };
        doc.saving = true;
        let core = self.ctx.core().clone();
        let id = self.sftp_id.clone();
        let path = doc.path.clone();
        let saved = content.clone();
        self.task = Some(run(
            &self.ctx,
            async move {
                core.sftp_write_file(&id, path.clone(), content).await?;
                Ok(core.sftp_stat_file(&id, path).await.ok())
            },
            window,
            cx,
            move |this: &mut Self, result, window, cx| {
                let current = this.editor.read(cx).value().to_string();
                let Some(doc) = this.doc.as_mut() else {
                    return;
                };
                doc.saving = false;
                match result {
                    Ok(stat) => {
                        doc.stat = stat;
                        // Typing during the save leaves the buffer dirty.
                        doc.dirty = current != saved;
                        doc.original = saved;
                        let name = display_name(&doc.name);
                        toast(window, cx, format!("Saved {name}"), ToastKind::Success);
                    }
                    Err(e) => toast(window, cx, format!("Save failed: {e}"), ToastKind::Error),
                }
                cx.notify();
            },
        ));
        cx.notify();
    }

    /// The close button and Escape. `onEditorClose`.
    fn close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.request_hide(window, cx, |_, _, _| {});
    }

    fn toggle_wrap(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.word_wrap = !self.word_wrap;
        let wrap = self.word_wrap;
        self.editor
            .update(cx, |s, cx| s.set_soft_wrap(wrap, window, cx));
        cx.notify();
    }

    fn on_key(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let ks = &event.keystroke;
        if ks.key == "s" && (ks.modifiers.control || ks.modifiers.platform) {
            cx.stop_propagation();
            self.save(window, cx);
        } else if ks.key == "escape" {
            cx.stop_propagation();
            self.close(window, cx);
        }
    }
}

impl Render for FloatingEditor {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(doc) = self.doc.as_ref() else {
            return div().into_any_element();
        };
        // Drawn by the window whatever is showing: hide with the panel. The
        // buffer, place and size are kept.
        if !self.ctx.is_shown(cx) {
            return div().into_any_element();
        }
        let p = *palette(cx);
        let origin = self.origin.unwrap_or_default();
        let (w, h) = (self.width, self.height);
        let dirty = doc.dirty;
        let saving = doc.saving;

        let title = h_flex()
            .id("sftp-editor-title")
            .flex_none()
            .px_3()
            .py_1p5()
            .gap_2()
            .justify_between()
            .border_b_1()
            .border_color(p.line)
            .bg(p.raised)
            .cursor_move()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, e: &MouseDownEvent, _, _| {
                    if let Some(origin) = this.origin {
                        this.move_from = Some((e.position, origin));
                    }
                }),
            )
            .on_drag(MoveEditor, |_, _, _, cx| cx.new(|_| Ghost))
            .child(
                h_flex()
                    .flex_1()
                    .min_w_0()
                    .gap_1()
                    .text_xs()
                    .text_color(p.ink)
                    .child(
                        div()
                            .truncate()
                            .child(SharedString::from(display_name(&doc.name))),
                    )
                    .when(dirty, |d| {
                        d.child(div().text_color(p.warn).child("\u{25CF}"))
                    }),
            )
            .child(
                h_flex()
                    .flex_none()
                    .gap_1p5()
                    .child(
                        Button::new("sftp-editor-wrap")
                            .ghost()
                            .xsmall()
                            .icon(IconName::TextWrap)
                            .label("Wrap")
                            .tooltip("Toggle word wrap")
                            .selected(self.word_wrap)
                            .on_click(
                                cx.listener(|this, _, window, cx| this.toggle_wrap(window, cx)),
                            ),
                    )
                    .child(
                        Button::new("sftp-editor-save")
                            .success()
                            .xsmall()
                            .label(if saving { "Saving..." } else { "Save" })
                            .disabled(saving || !dirty)
                            .on_click(cx.listener(|this, _, window, cx| this.save(window, cx))),
                    )
                    .child(
                        IconButton::new("sftp-editor-close", IconName::X, "Close editor")
                            .on_click(cx.listener(|this, _, window, cx| this.close(window, cx))),
                    ),
            );

        let body = if doc.loading {
            div()
                .flex_1()
                .flex()
                .items_center()
                .justify_center()
                .text_sm()
                .text_color(p.ink_2)
                .child("Loading...")
                .into_any_element()
        } else {
            div()
                .flex_1()
                .min_h_0()
                .bg(p.canvas)
                .child(
                    Editor::new(&self.editor)
                        .h_full()
                        .bordered(false)
                        .appearance(false),
                )
                .into_any_element()
        };

        let footer = h_flex()
            .flex_none()
            .px_3()
            .py_1p5()
            .justify_between()
            .border_t_1()
            .border_color(p.line)
            .text_color(p.ink_3)
            .child(text_2xs(div()).child(format!("{} chars", doc.chars)))
            .child(if dirty {
                text_2xs(div()).text_color(p.warn).child("Unsaved changes")
            } else {
                text_2xs(div()).child("Saved")
            });

        let grip = div()
            .id("sftp-editor-resize")
            .absolute()
            .bottom_0()
            .right_0()
            .size(px(12.))
            .border_r_2()
            .border_b_2()
            .border_color(p.ink_3)
            .cursor(CursorStyle::ResizeUpLeftDownRight)
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, e: &MouseDownEvent, _, _| {
                    this.resize_from = Some((e.position, (this.width, this.height)));
                }),
            )
            .on_drag(ResizeEditor, |_, _, _, cx| cx.new(|_| Ghost));

        let frame = v_flex()
            .id("sftp-editor")
            .relative()
            .occlude()
            .w(px(w))
            .h(px(h))
            .bg(p.surface)
            .border_1()
            .border_color(p.line)
            .rounded_sm()
            .shadow_xl()
            .capture_key_down(cx.listener(Self::on_key))
            .on_drag_move(cx.listener(|this, e: &DragMoveEvent<MoveEditor>, _, cx| {
                if let Some((mouse, origin)) = this.move_from {
                    let d = e.event.position - mouse;
                    this.origin = Some(point(
                        (origin.x + d.x).max(px(0.)),
                        (origin.y + d.y).max(px(0.)),
                    ));
                    cx.notify();
                }
            }))
            .on_drag_move(cx.listener(|this, e: &DragMoveEvent<ResizeEditor>, _, cx| {
                if let Some((mouse, (w0, h0))) = this.resize_from {
                    let d = e.event.position - mouse;
                    this.width = (w0 + f32::from(d.x)).max(MIN_W);
                    this.height = (h0 + f32::from(d.y)).max(MIN_H);
                    cx.notify();
                }
            }))
            .child(title)
            .child(body)
            .child(footer)
            .child(grip);

        anchored().position(origin).child(frame).into_any_element()
    }
}
