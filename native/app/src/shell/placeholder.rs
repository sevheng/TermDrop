//! What a tab shows until the integration step supplies real content: the
//! tab's kind and title, and where its real view will come from.

use gpui_kit::assets::IconName;
use gpui_kit::component::{Icon, Sizable as _, v_flex};
use gpui_kit::{
    AnyView, App, AppContext as _, Context, IntoElement, ParentElement, Render, SharedString,
    Styled, Window, div,
};

use crate::app::tab_kind::TabKind;
use crate::app::tabs::Tab;
use crate::ui::theme::palette;

pub struct TabPlaceholder {
    kind: TabKind,
    title: SharedString,
}

impl TabPlaceholder {
    pub fn view(tab: &Tab, cx: &mut App) -> AnyView {
        let kind = tab.kind;
        let title = SharedString::from(tab.title.clone());
        cx.new(|_| Self { kind, title }).into()
    }
}

impl Render for TabPlaceholder {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = *palette(cx);
        let (icon, what) = match self.kind {
            TabKind::Ssh => (IconName::Terminal, "The SSH terminal for this host"),
            TabKind::Local => (IconName::SquareTerminal, "A shell on this machine"),
            TabKind::Mongodb => (IconName::Database, "The MongoDB browser"),
            TabKind::Redis => (IconName::Layers, "The Redis browser"),
            TabKind::Vnc => (IconName::Monitor, "The remote desktop"),
        };
        v_flex()
            .size_full()
            .items_center()
            .justify_center()
            .gap_2()
            .bg(p.canvas)
            .child(Icon::new(icon).large().text_color(p.ink_3))
            .child(div().text_sm().text_color(p.ink).child(self.title.clone()))
            .child(
                div()
                    .text_xs()
                    .text_color(p.ink_2)
                    .child(format!("{what} renders here once it is wired in.")),
            )
    }
}
