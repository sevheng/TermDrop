//! The one empty-and-loading block. Port of `EmptyState.vue`, over GPUI
//! Kit's `Empty`.
//!
//! The three states must stay distinguishable: "nothing here yet" is not
//! "nothing matched" is not "still loading". Each has its own constructor,
//! and the two that describe a list with nothing in it *require* the action
//! that fixes it (add one; clear the search), because an empty state that
//! offers nothing to do is a dead end.

use std::rc::Rc;

use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::empty::{
    Empty, EmptyContent, EmptyDescription, EmptyHeader, EmptyMedia, EmptyMediaVariant, EmptyTitle,
};
use gpui_kit::component::spinner::Spinner;
use gpui_kit::component::{Icon, Sizable};
use gpui_kit::{
    App, ClickEvent, IntoElement, ParentElement, RenderOnce, SharedString, Styled, Window,
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
            EmptyKind::Empty | EmptyKind::Filtered => {
                let icon = self.icon.unwrap_or_else(|| {
                    Icon::new(match self.kind {
                        EmptyKind::Filtered => IconName::SearchX,
                        _ => IconName::Inbox,
                    })
                });
                EmptyMedia::new()
                    .with_variant(EmptyMediaVariant::Icon)
                    .child(icon.text_color(p.ink_3))
            }
        };
        let mut header = EmptyHeader::new().media(media).title(
            EmptyTitle::new()
                .text_sm()
                .text_color(p.ink)
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
        if let Some((label, on_action)) = self.action {
            empty = empty.content(
                EmptyContent::new().child(
                    Button::new(SharedString::from(format!("{}-action", self.id)))
                        .primary()
                        .small()
                        .label(label)
                        .on_click(move |e, w, cx| on_action(e, w, cx)),
                ),
            );
        }
        empty
    }
}
