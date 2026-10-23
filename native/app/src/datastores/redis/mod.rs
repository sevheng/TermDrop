//! The Redis tab: port of `RedisPanel.vue`, `RedisKeyList.vue`,
//! `RedisKeyTree.vue`, `RedisValueView.vue`, `RedisRestoreDialog.vue` and
//! their composables. Built against the contract in `../README.md`.

use gpui_kit::{AnyView, App, Window};

use super::ServiceTabContext;

/// The tab's view, built once when the tab opens and dropped when it closes.
pub fn build(ctx: ServiceTabContext, window: &mut Window, cx: &mut App) -> AnyView {
    super::placeholder::build(ctx, window, cx)
}
