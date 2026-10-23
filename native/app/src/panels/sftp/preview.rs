//! The file preview dialog. Port of `FilePreviewDialog.vue`: text with line
//! numbers and a find box, images shown whole, and a "cannot be previewed"
//! page with a download button for the binary types it knows by extension.
//! Text and images are read over SFTP (`sftp_read_file`, at most 5 MB, and
//! `sftp_read_file_base64`, at most 10 MB); binaries are never read.

use std::rc::Rc;
use std::sync::Arc;

use base64::Engine as _;
use gpui_kit::assets::IconName;
use gpui_kit::component::WindowExt as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Editor, EditorState, Input, InputEvent, InputState};
use gpui_kit::component::spinner::Spinner;
use gpui_kit::component::{Icon, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    App, AppContext as _, Context, Entity, Image, ImageFormat, IntoElement, ObjectFit,
    ParentElement, Render, SharedString, Styled, StyledImage as _, Subscription, Task, Window, div,
    img, px,
};

use super::model::{ImageKind, PreviewKind, match_label, preview_kind};
use super::run;
use crate::panels::PanelContext;
use crate::panels::common::{display_name, text_2xs};
use crate::ui::icon_button::IconButton;
use crate::ui::theme::palette;

type Action = Rc<dyn Fn(&mut Window, &mut App)>;

enum Body {
    Loading,
    Error(String),
    Text,
    Image(Arc<Image>),
    Binary,
}

pub struct FilePreview {
    kind: PreviewKind,
    body: Body,
    text: Entity<EditorState>,
    find: Entity<InputState>,
    query: String,
    on_edit: Action,
    on_download: Action,
    _task: Option<Task<()>>,
    _subscriptions: Vec<Subscription>,
}

/// Open the preview of `path`. `on_edit` and `on_download` run after the
/// dialog closes, as the Vue dialog's `edit` and `download` events did.
pub fn open_preview(
    ctx: &PanelContext,
    name: String,
    path: String,
    window: &mut Window,
    cx: &mut App,
    on_edit: impl Fn(&mut Window, &mut App) + 'static,
    on_download: impl Fn(&mut Window, &mut App) + 'static,
) {
    let kind = preview_kind(&name);
    let ctx = ctx.clone();
    let view = cx.new(|cx| FilePreview::new(&ctx, kind, path, window, cx, on_edit, on_download));
    let title = SharedString::from(format!(
        "{} {}",
        match kind {
            PreviewKind::Image(_) => "Image:",
            PreviewKind::Binary => "Binary:",
            PreviewKind::Text => "Preview:",
        },
        display_name(&name)
    ));
    window.open_dialog(cx, move |dialog, _, _| {
        dialog.title(title.clone()).w(px(640.)).child(view.clone())
    });
}

fn image_format(kind: ImageKind) -> ImageFormat {
    match kind {
        ImageKind::Png => ImageFormat::Png,
        ImageKind::Jpeg => ImageFormat::Jpeg,
        ImageKind::Gif => ImageFormat::Gif,
        ImageKind::Svg => ImageFormat::Svg,
        ImageKind::Webp => ImageFormat::Webp,
        ImageKind::Bmp => ImageFormat::Bmp,
        ImageKind::Ico => ImageFormat::Ico,
    }
}

impl FilePreview {
    fn new(
        ctx: &PanelContext,
        kind: PreviewKind,
        path: String,
        window: &mut Window,
        cx: &mut Context<Self>,
        on_edit: impl Fn(&mut Window, &mut App) + 'static,
        on_download: impl Fn(&mut Window, &mut App) + 'static,
    ) -> Self {
        let text = cx.new(|cx| {
            EditorState::new(window, cx)
                .soft_wrap(false)
                .searchable(false)
        });
        let find = cx.new(|cx| InputState::new(window, cx).placeholder("Find..."));
        let subscriptions = vec![cx.subscribe(&find, |this, input, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                this.query = input.read(cx).value().to_string();
                let q = this.query.trim().to_string();
                this.text.update(cx, |s, cx| {
                    if q.is_empty() {
                        s.close_search(cx);
                    } else {
                        s.set_search_query(q, true, cx);
                    }
                });
                cx.notify();
            }
        })];

        let id = ctx.sftp_session_id.clone().unwrap_or_default();
        let core = ctx.core().clone();
        let (body, task) = match kind {
            PreviewKind::Binary => (Body::Binary, None),
            PreviewKind::Image(image) => {
                let job = async move {
                    let b64 = core.sftp_read_file_base64(&id, path).await?;
                    base64::engine::general_purpose::STANDARD
                        .decode(b64.as_bytes())
                        .map_err(|e| e.to_string())
                };
                let task = run(ctx, job, window, cx, move |this: &mut Self, r, _, cx| {
                    this.body = match r {
                        Ok(bytes) => {
                            Body::Image(Arc::new(Image::from_bytes(image_format(image), bytes)))
                        }
                        Err(e) => Body::Error(e),
                    };
                    cx.notify();
                });
                (Body::Loading, Some(task))
            }
            PreviewKind::Text => {
                let job = async move { core.sftp_read_file(&id, path).await };
                let task = run(ctx, job, window, cx, |this: &mut Self, r, window, cx| {
                    match r {
                        Ok(content) => {
                            this.text
                                .update(cx, |s, cx| s.set_value(content, window, cx));
                            this.body = Body::Text;
                        }
                        Err(e) => this.body = Body::Error(e),
                    }
                    cx.notify();
                });
                (Body::Loading, Some(task))
            }
        };
        Self {
            kind,
            body,
            text,
            find,
            query: String::new(),
            on_edit: Rc::new(on_edit),
            on_download: Rc::new(on_download),
            _task: task,
            _subscriptions: subscriptions,
        }
    }

    fn step(&mut self, forward: bool, cx: &mut Context<Self>) {
        self.text.update(cx, |s, cx| {
            if forward {
                s.next_search_match(cx);
            } else {
                s.previous_search_match(cx);
            }
        });
        cx.notify();
    }

    fn render_find(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let p = *palette(cx);
        let session = self.text.read(cx).search_session();
        let count = session.matcher.len();
        let current = session.matcher.current_match_index();
        let searching = !self.query.trim().is_empty();
        h_flex()
            .gap_1()
            .child(
                div().w(px(150.)).child(
                    Input::new(&self.find)
                        .xsmall()
                        .prefix(Icon::new(IconName::Search).xsmall().text_color(p.ink_2)),
                ),
            )
            .when(searching, |d| {
                d.child(
                    text_2xs(div())
                        .text_color(p.ink_2)
                        .child(match_label(current, count)),
                )
            })
            .when(searching && count > 0, |d| {
                d.child(
                    IconButton::new("preview-prev", IconName::ChevronUp, "Previous")
                        .on_click(cx.listener(|this, _, _, cx| this.step(false, cx))),
                )
                .child(
                    IconButton::new("preview-next", IconName::ChevronDown, "Next")
                        .on_click(cx.listener(|this, _, _, cx| this.step(true, cx))),
                )
            })
    }

    fn close_then(action: Action) -> impl Fn(&gpui_kit::ClickEvent, &mut Window, &mut App) {
        move |_, window, cx| {
            window.close_dialog(cx);
            action(window, cx);
        }
    }
}

impl Render for FilePreview {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = *palette(cx);
        let is_text = self.kind == PreviewKind::Text;
        let toolbar = h_flex()
            .gap_1p5()
            .justify_end()
            .when(is_text, |d| d.child(self.render_find(cx)))
            .child(
                IconButton::new("preview-download", IconName::Download, "Download")
                    .on_click(Self::close_then(self.on_download.clone())),
            )
            .when(is_text, |d| {
                d.child(
                    Button::new("preview-edit")
                        .primary()
                        .xsmall()
                        .label("Edit")
                        .on_click(Self::close_then(self.on_edit.clone())),
                )
            });

        let center = || v_flex().size_full().items_center().justify_center().p_4();
        let content = match &self.body {
            Body::Loading => center()
                .child(
                    h_flex()
                        .gap_2()
                        .text_sm()
                        .text_color(p.ink_2)
                        .child(Spinner::new().small())
                        .child("Loading..."),
                )
                .into_any_element(),
            Body::Error(e) => center()
                .text_sm()
                .text_color(p.bad)
                .child(div().text_center().child(SharedString::from(e.clone())))
                .into_any_element(),
            Body::Image(image) => center()
                .child(
                    img(image.clone())
                        .max_w_full()
                        .max_h_full()
                        .object_fit(ObjectFit::Contain),
                )
                .into_any_element(),
            Body::Binary => center()
                .gap_1()
                .child(
                    Icon::new(IconName::FileArchive)
                        .size(px(48.))
                        .text_color(p.ink_3),
                )
                .child(
                    div()
                        .mt_2()
                        .text_sm()
                        .text_color(p.ink)
                        .child("Binary file"),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(p.ink_2)
                        .child("This file cannot be previewed."),
                )
                .child(
                    div().mt_3().child(
                        Button::new("preview-binary-download")
                            .primary()
                            .small()
                            .label("Download File")
                            .on_click(Self::close_then(self.on_download.clone())),
                    ),
                )
                .into_any_element(),
            Body::Text => div()
                .size_full()
                .bg(p.canvas)
                .child(
                    Editor::new(&self.text)
                        .readonly(true)
                        .h_full()
                        .bordered(false),
                )
                .into_any_element(),
        };

        v_flex().h(px(420.)).gap_2().child(toolbar).child(
            div()
                .flex_1()
                .min_h_0()
                .overflow_hidden()
                .border_1()
                .border_color(p.line)
                .rounded_sm()
                .child(content),
        )
    }
}
