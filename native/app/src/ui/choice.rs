//! An entry for GPUI Kit's `Select`: a value, its label and an optional hint
//! drawn after it in a quieter tone. Port of `SelectMenu.vue`'s
//! `{ value, label, hint }` options, where the hint is what tells two hosts
//! with the same name apart (`deploy@10.0.0.4`).

use gpui_kit::component::h_flex;
use gpui_kit::component::select::SelectItem;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{App, IntoElement, ParentElement, SharedString, Styled, Window, div};

use super::theme::palette;

#[derive(Debug, Clone, PartialEq)]
pub struct Choice<V> {
    pub value: V,
    pub label: SharedString,
    pub hint: Option<SharedString>,
}

impl<V> Choice<V> {
    pub fn new(value: V, label: impl Into<SharedString>) -> Self {
        Self {
            value,
            label: label.into(),
            hint: None,
        }
    }

    pub fn hint(mut self, hint: Option<impl Into<SharedString>>) -> Self {
        self.hint = hint.map(Into::into);
        self
    }
}

impl<V: Clone + PartialEq + 'static> SelectItem for Choice<V> {
    type Value = V;

    fn title(&self) -> SharedString {
        self.label.clone()
    }

    fn value(&self) -> &V {
        &self.value
    }

    fn render(&self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let p = *palette(cx);
        h_flex()
            .gap_2()
            .child(self.label.clone())
            .when_some(self.hint.clone(), |row, hint| {
                row.child(div().text_xs().text_color(p.ink_3).child(hint))
            })
    }

    fn matches(&self, query: &str) -> bool {
        let q = query.to_lowercase();
        self.label.to_lowercase().contains(&q)
            || self
                .hint
                .as_ref()
                .is_some_and(|h| h.to_lowercase().contains(&q))
    }
}

/// The index of `value` in `choices`, for a `SelectState`'s initial pick.
pub fn index_of<V: PartialEq>(choices: &[Choice<V>], value: &V) -> Option<usize> {
    choices.iter().position(|c| &c.value == value)
}
