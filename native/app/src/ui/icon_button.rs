//! An icon-only button that cannot be built without a label.
//!
//! Port of `IconButton.vue`'s one rule: the label is required, and it becomes
//! both the tooltip and the accessible name. An icon alone names nothing for
//! a screen reader and is a guess for everyone else, so the constructor takes
//! the label rather than offering it as an optional builder.

use std::rc::Rc;

use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::{Disableable, Icon, Selectable, Sizable};
use gpui_kit::{
    App, ClickEvent, ElementId, Hsla, IntoElement, RenderOnce, SharedString, Styled, Window,
};

type OnClick = Rc<dyn Fn(&ClickEvent, &mut Window, &mut App)>;

#[derive(IntoElement)]
pub struct IconButton {
    id: ElementId,
    icon: Icon,
    label: SharedString,
    on_click: Option<OnClick>,
    tone: Option<Hsla>,
    active: bool,
    disabled: bool,
    pending: bool,
}

impl IconButton {
    /// `label` says what the button does ("Close tab", "Add a host"). It is
    /// required and must not be empty.
    pub fn new(
        id: impl Into<ElementId>,
        icon: impl Into<Icon>,
        label: impl Into<SharedString>,
    ) -> Self {
        let label = label.into();
        debug_assert!(!label.trim().is_empty(), "an IconButton needs a label");
        Self {
            id: id.into(),
            icon: icon.into(),
            label,
            on_click: None,
            tone: None,
            active: false,
            disabled: false,
            pending: false,
        }
    }

    pub fn on_click(mut self, f: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static) -> Self {
        self.on_click = Some(Rc::new(f));
        self
    }

    /// A token colour for the icon (`warn` for a favourite star, `bad` for a
    /// delete). Never a literal colour: callers pass a `Palette` field.
    pub fn tone(mut self, tone: Hsla) -> Self {
        self.tone = Some(tone);
        self
    }

    /// Pressed-in, for toggles and for a button whose menu is open.
    pub fn active(mut self, active: bool) -> Self {
        self.active = active;
        self
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    /// Busy: a spinner in place of the icon, and not clickable, while what
    /// it started runs (a refresh). Vue's `pending`.
    pub fn pending(mut self, pending: bool) -> Self {
        self.pending = pending;
        self
    }

    pub fn label(&self) -> &SharedString {
        &self.label
    }
}

impl RenderOnce for IconButton {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        let icon = match self.tone {
            Some(tone) => self.icon.text_color(tone),
            None => self.icon,
        };
        let mut button = Button::new(self.id)
            .ghost()
            .small()
            .icon(icon)
            .tooltip(self.label.clone())
            .accessibility_label(self.label)
            .selected(self.active)
            .loading(self.pending)
            .disabled(self.disabled || self.pending);
        if let Some(on_click) = self.on_click {
            button = button.on_click(move |e, w, cx| on_click(e, w, cx));
        }
        button
    }
}
