//! A small labelled tag for a header's badge slot ("via bastion",
//! "cluster"), with a tooltip saying what it means. `PanelHeader::badge`
//! takes any element; this is the one to give it when the badge is a word
//! rather than a control.

use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::{
    App, InteractiveElement, IntoElement, ParentElement, RenderOnce, SharedString,
    StatefulInteractiveElement, Styled, Window, div, px,
};

use super::theme::palette;
use crate::panels::common::text_2xs;

/// Which tone the tag carries. Each pairs a text token with its fill, so a
/// tag never names a colour.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TagTone {
    /// A fact about the connection: accent text on the active fill.
    Accent,
    /// A limit the user should know about: warn text on the warn fill.
    Warn,
    /// Something wrong: bad text on the bad fill.
    Bad,
    /// Plain: secondary text on the raised fill.
    Neutral,
}

#[derive(IntoElement)]
pub struct Tag {
    id: SharedString,
    label: SharedString,
    tip: Option<SharedString>,
    tone: TagTone,
}

impl Tag {
    pub fn new(id: impl Into<SharedString>, label: impl Into<SharedString>) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            tip: None,
            tone: TagTone::Neutral,
        }
    }

    pub fn tone(mut self, tone: TagTone) -> Self {
        self.tone = tone;
        self
    }

    /// What the tag means, on hover.
    pub fn tooltip(mut self, tip: impl Into<SharedString>) -> Self {
        self.tip = Some(tip.into());
        self
    }
}

impl RenderOnce for Tag {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let p = *palette(cx);
        let (text, fill) = match self.tone {
            TagTone::Accent => (p.accent, p.active),
            TagTone::Warn => (p.warn, p.warn_bg),
            TagTone::Bad => (p.bad, p.bad_bg),
            TagTone::Neutral => (p.ink_2, p.raised),
        };
        let tag = text_2xs(div())
            .id(self.id)
            .flex_none()
            .px(px(6.))
            .py(px(2.))
            .rounded_sm()
            .bg(fill)
            .text_color(text)
            .child(self.label);
        match self.tip {
            Some(tip) => tag
                .tooltip(move |window, cx| Tooltip::new(tip.clone()).build(window, cx))
                .into_any_element(),
            None => tag.into_any_element(),
        }
    }
}
