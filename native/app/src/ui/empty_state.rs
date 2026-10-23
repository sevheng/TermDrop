//! The one empty, loading and error block. Port of `EmptyState.vue`, over
//! GPUI Kit's `Empty`.
//!
//! The states must stay distinguishable: "nothing here yet" is not "nothing
//! matched" is not "still loading" is not "it failed". Each has its own
//! constructor, and every one but loading *requires* the way out: the action
//! that fixes it (add one, clear the search, retry), or for a fix that only
//! the user can make outside the app, [`EmptyState::steps`] with the
//! instructions, because an empty state that offers nothing to do is a dead
//! end.

use std::rc::Rc;

use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::empty::{
    Empty, EmptyContent, EmptyDescription, EmptyHeader, EmptyMedia, EmptyMediaVariant, EmptyTitle,
};
use gpui_kit::component::spinner::Spinner;
use gpui_kit::component::{Disableable, Icon, Sizable};
use gpui_kit::{
    AnyElement, App, ClickEvent, IntoElement, ParentElement, RenderOnce, SharedString, Styled,
    Window,
};

use super::theme::palette;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EmptyKind {
    /// Nothing exists yet. Offers the action that creates the first one.
    Empty,
    /// Things exist, and a filter hides all of them. Offers to clear it.
    Filtered,
    /// Not known yet. Offers nothing, because there is nothing to fix.
    Loading,
    /// Finding out failed. Offers the retry, and says why in the hint.
    Error,
}

type OnAction = Rc<dyn Fn(&ClickEvent, &mut Window, &mut App)>;

#[derive(IntoElement)]
pub struct EmptyState {
    kind: EmptyKind,
    id: SharedString,
    icon: Option<Icon>,
    title: SharedString,
    hint: Option<SharedString>,
    action: Option<(SharedString, OnAction)>,
    /// Set while the action runs: the button is disabled, shows a spinner
    /// and this label (Vue's `actionPending` + `pendingLabel`).
    pending: Option<SharedString>,
    /// Under the hint, above the action (Vue's default slot).
    content: Vec<AnyElement>,
    compact: bool,
}

impl EmptyState {
    fn new(kind: EmptyKind, id: impl Into<SharedString>, title: impl Into<SharedString>) -> Self {
        Self {
            kind,
            id: id.into(),
            icon: None,
            title: title.into(),
            hint: None,
            action: None,
            pending: None,
            content: Vec::new(),
            compact: false,
        }
    }

    pub fn empty(
        id: impl Into<SharedString>,
        title: impl Into<SharedString>,
        action_label: impl Into<SharedString>,
        on_action: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        let mut s = Self::new(EmptyKind::Empty, id, title);
        s.action = Some((action_label.into(), Rc::new(on_action)));
        s
    }

    pub fn filtered(
        id: impl Into<SharedString>,
        title: impl Into<SharedString>,
        action_label: impl Into<SharedString>,
        on_action: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        let mut s = Self::new(EmptyKind::Filtered, id, title);
        s.action = Some((action_label.into(), Rc::new(on_action)));
        s
    }

    pub fn loading(id: impl Into<SharedString>, title: impl Into<SharedString>) -> Self {
        Self::new(EmptyKind::Loading, id, title)
    }

    /// Something failed: the title in the bad tone, the error as the
    /// [`hint`](Self::hint), and the retry. Vue's `state="error"`.
    pub fn error(
        id: impl Into<SharedString>,
        title: impl Into<SharedString>,
        action_label: impl Into<SharedString>,
        on_action: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        let mut s = Self::new(EmptyKind::Error, id, title);
        s.action = Some((action_label.into(), Rc::new(on_action)));
        s
    }

    /// Nothing to show, and the fix is something the user does outside the
    /// app (start a daemon, join a group): `how` says what, in place of an
    /// action button. Vue's empty states with only slot content.
    pub fn steps(
        id: impl Into<SharedString>,
        title: impl Into<SharedString>,
        how: impl IntoElement,
    ) -> Self {
        let mut s = Self::new(EmptyKind::Empty, id, title);
        s.content.push(how.into_any_element());
        s
    }

    /// More content under the hint and above the action.
    pub fn child(mut self, child: impl IntoElement) -> Self {
        self.content.push(child.into_any_element());
        self
    }

    /// While `pending` is `Some`, the action is disabled and shows a
    /// spinner and that label instead of its own.
    pub fn pending(mut self, pending: Option<impl Into<SharedString>>) -> Self {
        self.pending = pending.map(Into::into);
        self
    }

    pub fn icon(mut self, icon: impl Into<Icon>) -> Self {
        self.icon = Some(icon.into());
        self
    }

    pub fn hint(mut self, hint: impl Into<SharedString>) -> Self {
        self.hint = Some(hint.into());
        self
    }

    /// Tighter padding and no frame, for a sidebar list.
    pub fn compact(mut self) -> Self {
        self.compact = true;
        self
    }

    pub fn kind(&self) -> EmptyKind {
        self.kind
    }
}

impl RenderOnce for EmptyState {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let p = *palette(cx);
        // Each state has its own default picture, so the three read
        // differently even before the words are read.
        let media = match self.kind {
            EmptyKind::Loading => EmptyMedia::new().child(Spinner::new().color(p.ink_3)),
            EmptyKind::Empty | EmptyKind::Filtered | EmptyKind::Error => {
                let icon = self.icon.unwrap_or_else(|| {
                    Icon::new(match self.kind {
                        EmptyKind::Filtered => IconName::SearchX,
                        EmptyKind::Error => IconName::CircleAlert,
                        _ => IconName::Inbox,
                    })
                });
                let tone = if self.kind == EmptyKind::Error {
                    p.bad
                } else {
                    p.ink_3
                };
                EmptyMedia::new()
                    .with_variant(EmptyMediaVariant::Icon)
                    .child(icon.text_color(tone))
            }
        };
        let title_tone = if self.kind == EmptyKind::Error {
            p.bad
        } else {
            p.ink
        };
        let mut header = EmptyHeader::new().media(media).title(
            EmptyTitle::new()
                .text_sm()
                .text_color(title_tone)
                .child(self.title),
        );
        if let Some(hint) = self.hint {
            header = header.description(
                EmptyDescription::new()
                    .text_xs()
                    .text_color(p.ink_2)
                    .child(hint),
            );
        }
        let mut empty = Empty::new().header(header);
        if self.compact {
            empty = empty.p_3().gap_2().border_0();
        }
        let mut content = self.content;
        if let Some((label, on_action)) = self.action {
            let pending = self.pending.is_some();
            content.push(
                Button::new(SharedString::from(format!("{}-action", self.id)))
                    .primary()
                    .small()
                    .label(self.pending.unwrap_or(label))
                    .loading(pending)
                    .disabled(pending)
                    .on_click(move |e, w, cx| on_action(e, w, cx))
                    .into_any_element(),
            );
        }
        if !content.is_empty() {
            empty = empty.content(EmptyContent::new().children(content));
        }
        empty
    }
}
