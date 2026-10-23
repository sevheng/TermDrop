//! A tab the native app cannot fill. MongoDB, Redis and VNC tabs are built
//! by `crate::datastores`; their notices here are reached only if the model
//! is gone. It says so, and why, rather than showing a
//! blank pane or a view that pretends to work.

use gpui_kit::assets::IconName;
use gpui_kit::component::Icon;
use gpui_kit::{
    Context, IntoElement, ParentElement, Render, SharedString, Styled, WeakEntity, Window, div, px,
};

use crate::app::AppModel;
use crate::app::tab_kind::TabKind;
use crate::app::tabs::Tab;
use crate::ui::empty_state::EmptyState;
use crate::ui::theme::palette;

pub struct Unsupported {
    tab_id: String,
    kind: TabKind,
    model: Option<WeakEntity<AppModel>>,
}

impl Unsupported {
    pub fn new(tab: &Tab, model: Option<WeakEntity<AppModel>>) -> Self {
        Self {
            tab_id: tab.id.clone(),
            kind: tab.kind,
            model,
        }
    }
}

/// Title, reason and icon for a kind that is not ported.
pub(super) fn notice(kind: TabKind) -> (&'static str, &'static str, IconName) {
    match kind {
        TabKind::Mongodb => (
            "MongoDB is not in the native app yet",
            "The MongoDB browser, backup and restore are still being ported. Open this host in \
             the TermDrop app for now; it reads the same saved hosts.",
            IconName::Database,
        ),
        TabKind::Redis => (
            "Redis is not in the native app yet",
            "The Redis browser and backups are still being ported. Open this host in the \
             TermDrop app for now; it reads the same saved hosts.",
            IconName::Layers,
        ),
        TabKind::Vnc => (
            "Remote desktop is not in the native app yet",
            "The VNC viewer is still being ported. Open this host in the TermDrop app for now; \
             it reads the same saved hosts.",
            IconName::Monitor,
        ),
        // Terminals are always built; reaching here means the tab lost its
        // host, which only a bug could do.
        TabKind::Ssh | TabKind::Local => (
            "This tab cannot be opened",
            "Its host is no longer in the host list.",
            IconName::Terminal,
        ),
    }
}

impl Render for Unsupported {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = *palette(cx);
        let (title, reason, icon) = notice(self.kind);
        let model = self.model.clone();
        let tab_id = self.tab_id.clone();
        div()
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .bg(p.canvas)
            .child(
                div().w(px(440.)).child(
                    EmptyState::empty(
                        SharedString::from(format!("unsupported-{}", self.tab_id)),
                        title,
                        "Close tab",
                        move |_, _, cx| {
                            // Nothing is connected behind this tab, so it
                            // closes without asking.
                            if let Some(model) = model.as_ref().and_then(WeakEntity::upgrade) {
                                model.update(cx, |m, cx| {
                                    m.close_tab(&tab_id, cx);
                                });
                            }
                        },
                    )
                    .icon(Icon::new(icon))
                    .hint(reason),
                ),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_unported_kind_says_why() {
        for kind in [TabKind::Mongodb, TabKind::Redis, TabKind::Vnc] {
            let (title, reason, _) = notice(kind);
            assert!(title.contains("not in the native app yet"), "{kind:?}");
            assert!(!reason.is_empty());
        }
    }
}
