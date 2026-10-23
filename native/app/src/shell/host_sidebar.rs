//! The host list. Port of `HostSidebar.vue` + `HostRow.vue`: search, grouped
//! or flat view, collapsible groups, favourites, keyboard navigation, the
//! header's add / import / export menus, each row's hover buttons and the
//! host, group and empty-area context menus.
//!
//! The rules live in the pure modules (`host_grouping`, `list_nav`,
//! `host_row_state`, `host_menu`); this file only lays them out. The dialogs
//! it opens are `host_editor`, `group_dialog` and `ssh_import`. GPUI Kit's `Sidebar` is
//! not used for the list itself: its items are a menu model, and this list is
//! a keyboard-navigable tree with per-row state and context menus.

use std::collections::HashSet;

use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::menu::{ContextMenuExt as _, DropdownMenu as _, PopupMenu, PopupMenuItem};
use gpui_kit::component::spinner::Spinner;
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{Icon, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    Anchor, AnyElement, App, AppContext as _, ClickEvent, Context, Entity, FocusHandle, Focusable,
    FontWeight, InteractiveElement, IntoElement, KeyDownEvent, ParentElement, Render, ScrollHandle,
    SharedString, StatefulInteractiveElement, Styled, Subscription, Window, div, px,
};

use super::group_dialog::{GroupDialogMode, open_group_dialog};
use super::host_actions::{
    confirm_delete_group, confirm_delete_host, export_hosts, import_json, move_to_group,
};
use super::host_editor::open_host_editor;
use super::ssh_import::open_ssh_import;
use crate::app::display::{connected_rail_title, jump_label, subtitle_title};
use crate::app::group_accent::group_accent_index;
use crate::app::host_form::EditorKind;
use crate::app::host_grouping::{filter_hosts, group_hosts};
use crate::app::host_kind::{HostKind, IconTone};
use crate::app::host_menu::{
    EmptyMenuItem, GroupMenuItem, HostMenuItem, add_menu, empty_menu, group_menu, host_menu,
};
use crate::app::host_row_state::{HostRowState, format_age};
use crate::app::list_nav::{
    HOST_ROW_H, NavIntent, Row, dom_key, first_row, last_row, nav_intent, next_row, page_row,
    visible_rows,
};
use crate::app::prefs::ViewMode;
use crate::app::{AppEvent, AppModel};
use crate::backend::HostSummary;
use crate::ui::empty_state::EmptyState;
use crate::ui::icon_button::IconButton;
use crate::ui::panel_header::PanelHeader;
use crate::ui::theme::{Palette, palette};

/// What a host row carries while it is dragged to a group header, and the
/// ghost under the pointer: the kind (or `user@host`) dimmed, then the name,
/// as `HostRow.vue`'s drag image.
#[derive(Clone)]
struct DraggedHost {
    id: i64,
    name: String,
    kind_label: String,
}

impl Render for DraggedHost {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = *palette(cx);
        h_flex()
            .gap_1()
            .px_2()
            .py(px(2.))
            .rounded_sm()
            .bg(p.overlay)
            .border_1()
            .border_color(p.line)
            .text_xs()
            .text_color(p.ink)
            .whitespace_nowrap()
            .child(div().opacity(0.6).child(self.kind_label.clone()))
            .child(
                div()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(self.name.clone()),
            )
    }
}

pub struct HostSidebar {
    model: Entity<AppModel>,
    search: Entity<InputState>,
    query: String,
    focused_host: Option<i64>,
    focus_handle: FocusHandle,
    scroll: ScrollHandle,
    _subscriptions: Vec<Subscription>,
}

impl Focusable for HostSidebar {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

/// The rows the arrow keys walk, in order. Favourites are shown above the
/// groups but are not part of the walk, as in the Vue sidebar: they repeat
/// hosts that the groups already contain.
type NavRows<'a> = Vec<Row<&'a HostSummary>>;

impl HostSidebar {
    pub fn new(model: Entity<AppModel>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let search = cx.new(|cx| InputState::new(window, cx).placeholder("Search hosts..."));
        let subscriptions = vec![
            cx.subscribe(&search, |this, input, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    this.query = input.read(cx).value().to_string();
                    cx.notify();
                }
            }),
            cx.observe(&model, |_, _, cx| cx.notify()),
            cx.subscribe(&model, |this, model, event: &AppEvent, cx| {
                // A host that left the list cannot keep the keyboard focus.
                if matches!(event, AppEvent::HostsChanged)
                    && let Some(id) = this.focused_host
                    && model.read(cx).host(id).is_none()
                {
                    this.focused_host = None;
                }
            }),
        ];
        Self {
            model,
            search,
            query: String::new(),
            focused_host: None,
            focus_handle: cx.focus_handle(),
            scroll: ScrollHandle::new(),
            _subscriptions: subscriptions,
        }
    }

    fn clear_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.search.update(cx, |s, cx| s.set_value("", window, cx));
        self.query.clear();
        cx.notify();
    }

    fn activate(&mut self, host_id: i64, cx: &mut Context<Self>) {
        self.focused_host = Some(host_id);
        self.model.update(cx, |m, cx| {
            m.activate_host(host_id, cx);
        });
    }

    /// Rows for keyboard navigation, as the eye sees them.
    fn nav_rows<'a>(model: &'a AppModel, query: &str) -> (NavRows<'a>, Vec<&'a HostSummary>) {
        let filtered = filter_hosts(model.hosts(), query);
        match model.view_mode() {
            ViewMode::Flat => (
                filtered
                    .iter()
                    .map(|&h| Row::Item {
                        item: h,
                        group: h.group.clone(),
                    })
                    .collect(),
                Vec::new(),
            ),
            ViewMode::Grouped => {
                let (favs, rest): (Vec<_>, Vec<_>) = filtered.into_iter().partition(|h| h.favorite);
                let groups = group_hosts(&rest, model.custom_groups());
                let collapsed = model.collapsed_groups();
                let rows = visible_rows(
                    groups.iter().map(|g| (g.name.as_str(), g.hosts.as_slice())),
                    &collapsed,
                );
                // Favourites hide while searching, so a match is shown once.
                let favs = if query.trim().is_empty() {
                    favs
                } else {
                    Vec::new()
                };
                (rows, favs)
            }
        }
    }

    fn on_key_down(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let ks = &event.keystroke;
        if ks.modifiers.control || ks.modifiers.alt || ks.modifiers.platform {
            return;
        }
        let Some(key) = dom_key(&ks.key) else { return };
        let Some(intent) = nav_intent(key, false, false) else {
            return;
        };
        let model = self.model.read(cx);
        let (rows, favs) = Self::nav_rows(model, &self.query);
        if rows.is_empty() {
            return;
        }
        let current = rows
            .iter()
            .position(|r| matches!(r, Row::Item { item, .. } if Some(item.id) == self.focused_host))
            .map_or(-1, |i| i as isize);
        let next = match intent {
            NavIntent::Move(dir) => next_row(&rows, current, dir),
            NavIntent::First => first_row(&rows),
            NavIntent::Last => last_row(&rows),
            NavIntent::Page(dir) => page_row(&rows, current, dir, 10),
            NavIntent::Activate => {
                if let Some(Row::Item { item, .. }) =
                    usize::try_from(current).ok().and_then(|i| rows.get(i))
                {
                    let id = item.id;
                    cx.stop_propagation();
                    self.activate(id, cx);
                }
                return;
            }
            NavIntent::Delete => {
                if let Some(Row::Item { item, .. }) =
                    usize::try_from(current).ok().and_then(|i| rows.get(i))
                {
                    let id = item.id;
                    cx.stop_propagation();
                    confirm_delete_host(window, cx, self.model.clone(), id);
                }
                return;
            }
            // The other tree intents have nothing to act on while focus is
            // only ever on a host row.
            _ => return,
        };
        if let Some(Row::Item { item, .. }) = usize::try_from(next).ok().and_then(|i| rows.get(i)) {
            self.focused_host = Some(item.id);
            // Rows are the scroll container's children, after the favourites
            // block (a header plus its rows) when that is shown.
            let offset = if favs.is_empty() { 0 } else { favs.len() + 1 };
            self.scroll.scroll_to_item(offset + next as usize);
            cx.stop_propagation();
            window.refresh();
            cx.notify();
        }
    }

    /// The header's "Add a host" button: a menu of the four editors.
    fn add_button(&self) -> impl IntoElement {
        let model = self.model.clone();
        const LABEL: &str = "Add a host";
        Button::new("add-host")
            .ghost()
            .small()
            .icon(Icon::new(IconName::Plus))
            .tooltip(LABEL)
            .accessibility_label(LABEL)
            .dropdown_menu_with_anchor(Anchor::TopRight, move |mut menu, _, cx| {
                let p = *palette(cx);
                for (kind, label) in add_menu() {
                    let model = model.clone();
                    let (icon, tone) = match kind {
                        EditorKind::Ssh => (IconName::Server, p.ink_2),
                        EditorKind::Mongo => (IconName::Database, p.mongo),
                        EditorKind::Redis => (IconName::Layers, p.redis),
                        EditorKind::Vnc => (IconName::Monitor, p.vnc),
                    };
                    menu = menu.item(
                        PopupMenuItem::new(label)
                            .icon(Icon::new(icon).text_color(tone))
                            .on_click(move |_, window, cx| {
                                open_host_editor(window, cx, model.clone(), kind, None, None)
                            }),
                    );
                }
                menu
            })
    }

    /// The header's import button: from `~/.ssh/config` or a JSON export.
    fn import_button(&self) -> impl IntoElement {
        let model = self.model.clone();
        const LABEL: &str = "Import hosts";
        Button::new("import-hosts")
            .ghost()
            .small()
            .icon(Icon::new(IconName::Download))
            .tooltip(LABEL)
            .accessibility_label(LABEL)
            .dropdown_menu_with_anchor(Anchor::TopRight, move |menu, _, _| {
                let m1 = model.clone();
                let m2 = model.clone();
                menu.item(
                    PopupMenuItem::new("From ~/.ssh/config")
                        .icon(IconName::FileTerminal)
                        .on_click(move |_, window, cx| open_ssh_import(window, cx, m1.clone())),
                )
                .item(
                    PopupMenuItem::new("From JSON file")
                        .icon(IconName::Download)
                        .on_click(move |_, window, cx| import_json(window, cx, m2.clone())),
                )
            })
    }

    /// The host row's menu, built from `host_menu` when it opens.
    fn host_context_menu(
        model: &Entity<AppModel>,
        sidebar: &gpui_kit::WeakEntity<Self>,
        id: i64,
        menu: PopupMenu,
        cx: &App,
    ) -> PopupMenu {
        let m = model.read(cx);
        let Some(host) = m.host(id).cloned() else {
            return menu;
        };
        let p = *palette(cx);
        let items = host_menu(&host, m.view_mode(), &m.group_names());
        let kind = host.kind;
        let mut menu = menu;
        for item in items {
            let label = item.label();
            let model = model.clone();
            menu = match item {
                HostMenuItem::Separator => menu.separator(),
                HostMenuItem::MoveToLabel => menu.label(label),
                HostMenuItem::Activate(_) => {
                    let sidebar = sidebar.clone();
                    menu.item(
                        PopupMenuItem::new(label)
                            .icon(Icon::new(IconName::Zap).text_color(p.accent))
                            .on_click(move |_, _, cx| {
                                if let Some(s) = sidebar.upgrade() {
                                    s.update(cx, |s, cx| s.activate(id, cx));
                                }
                            }),
                    )
                }
                HostMenuItem::OpenVnc => menu.item(
                    PopupMenuItem::new(label)
                        .icon(Icon::new(IconName::Monitor).text_color(p.vnc))
                        .on_click(move |_, _, cx| {
                            model.update(cx, |m, cx| {
                                m.open_vnc(id, cx);
                            });
                        }),
                ),
                HostMenuItem::Edit => menu.item(
                    PopupMenuItem::new(label)
                        .icon(Icon::new(IconName::Pencil).text_color(p.ink_3))
                        .on_click(move |_, window, cx| {
                            open_host_editor(
                                window,
                                cx,
                                model.clone(),
                                EditorKind::for_host(kind),
                                Some(id),
                                None,
                            )
                        }),
                ),
                HostMenuItem::ToggleFavorite { favorite } => menu.item(
                    PopupMenuItem::new(label)
                        .icon(Icon::new(IconName::Star).text_color(p.warn))
                        .on_click(move |_, _, cx| {
                            model.update(cx, |m, cx| m.set_favorite(id, !favorite, cx));
                        }),
                ),
                HostMenuItem::CopyName => {
                    menu.item(PopupMenuItem::new(label).icon(IconName::Copy).on_click(
                        move |_, _, cx| {
                            if let Some(h) = model.read(cx).host(id) {
                                cx.write_to_clipboard(gpui_kit::ClipboardItem::new_string(
                                    h.name.clone(),
                                ));
                            }
                        },
                    ))
                }
                HostMenuItem::Delete => menu.item(
                    PopupMenuItem::new(label)
                        .icon(Icon::new(IconName::Trash).text_color(p.bad))
                        .on_click(move |_, window, cx| {
                            confirm_delete_host(window, cx, model.clone(), id)
                        }),
                ),
                HostMenuItem::MoveTo(group) => menu.item(
                    PopupMenuItem::new(label)
                        .icon(Icon::new(IconName::Folder).text_color(p.ink_3))
                        .checked(group == host.group)
                        .on_click(move |_, window, cx| {
                            move_to_group(window, cx, &model, id, &group)
                        }),
                ),
            };
        }
        menu
    }

    /// The empty area's menu: a new group (grouped view) and a new host.
    fn empty_context_menu(model: &Entity<AppModel>, menu: PopupMenu, cx: &App) -> PopupMenu {
        let p = *palette(cx);
        let mut menu = menu;
        for item in empty_menu(model.read(cx).view_mode()) {
            let model = model.clone();
            menu = menu.item(match item {
                EmptyMenuItem::NewGroup => PopupMenuItem::new(item.label())
                    .icon(Icon::new(IconName::FolderPlus).text_color(p.accent))
                    .on_click(move |_, window, cx| {
                        open_group_dialog(window, cx, model.clone(), GroupDialogMode::Create)
                    }),
                EmptyMenuItem::AddHost => PopupMenuItem::new(item.label())
                    .icon(Icon::new(IconName::Plus).text_color(p.good))
                    .on_click(move |_, window, cx| {
                        open_host_editor(window, cx, model.clone(), EditorKind::Ssh, None, None)
                    }),
            });
        }
        menu
    }

    /// A group header's menu: add a host into it, rename it, delete it.
    fn group_context_menu(
        model: &Entity<AppModel>,
        name: &str,
        menu: PopupMenu,
        cx: &App,
    ) -> PopupMenu {
        let p = *palette(cx);
        let mut menu = menu;
        for (item, enabled) in group_menu(name) {
            let model = model.clone();
            let name = name.to_string();
            menu = menu.item(match item {
                GroupMenuItem::AddHost => PopupMenuItem::new(item.label())
                    .icon(Icon::new(IconName::Plus).text_color(p.good))
                    .on_click(move |_, window, cx| {
                        let group = (!name.is_empty()).then(|| name.clone());
                        open_host_editor(window, cx, model.clone(), EditorKind::Ssh, None, group)
                    }),
                GroupMenuItem::Rename => PopupMenuItem::new(item.label())
                    .icon(Icon::new(IconName::Pencil).text_color(p.ink_3))
                    .disabled(!enabled)
                    .on_click(move |_, window, cx| {
                        open_group_dialog(
                            window,
                            cx,
                            model.clone(),
                            GroupDialogMode::Rename(name.clone()),
                        )
                    }),
                GroupMenuItem::Delete => PopupMenuItem::new(item.label())
                    .icon(Icon::new(IconName::Trash).text_color(p.bad))
                    .disabled(!enabled)
                    .on_click(move |_, window, cx| {
                        confirm_delete_group(window, cx, model.clone(), name.clone())
                    }),
            });
        }
        menu
    }

    fn render_row(
        &self,
        host: &HostSummary,
        state: HostRowState,
        key: &str,
        now_ms: i64,
        p: &Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let id = host.id;
        let focused = self.focused_host == Some(id) && !state.active;
        let icon = match host.kind {
            HostKind::Mongodb => IconName::Database,
            HostKind::Redis => IconName::Layers,
            HostKind::Vnc => IconName::Monitor,
            HostKind::Ssh | HostKind::Local if host.mac => IconName::Apple,
            HostKind::Ssh | HostKind::Local => IconName::Server,
        };
        let tone = match host.kind.icon_tone() {
            IconTone::Ink3 => p.ink_3,
            IconTone::Redis => p.redis,
            IconTone::Mongo => p.mongo,
            IconTone::Vnc => p.vnc,
        };
        let subtitle: SharedString = host.subtitle.clone().into();
        let model_ref = self.model.read(cx);
        let jump = jump_label(
            host.jump_host_id,
            host.jump_host_id
                .and_then(|j| model_ref.host(j))
                .map(|h| h.name.as_str()),
        );
        let subtitle_tip: SharedString = subtitle_title(&subtitle, jump.as_deref()).into();
        let rail_tip: SharedString = connected_rail_title(state.tab_count).into();
        let grouped = model_ref.view_mode() == ViewMode::Grouped;
        let drag = DraggedHost {
            id,
            name: host.name.clone(),
            kind_label: match host.kind {
                HostKind::Mongodb => "MongoDB".into(),
                HostKind::Redis => "Redis".into(),
                HostKind::Vnc => "VNC".into(),
                HostKind::Ssh | HostKind::Local => format!("{}@{}", host.username, host.host),
            },
        };
        let age = if state.connected {
            String::new()
        } else {
            format_age(host.last_connected_at.as_deref(), now_ms)
        };
        let favorite = host.favorite;
        let kind = host.kind;
        let model = self.model.clone();
        let sidebar = cx.entity().downgrade();
        let group = SharedString::from(format!("host-row-{key}-{id}"));
        // The hover cluster's backing matches the row under it, so the
        // buttons never sit on top of text.
        let cluster_bg = if state.active { p.selected } else { p.raised };
        let (m_fav, m_edit, m_del) = (model.clone(), model.clone(), model.clone());

        div()
            .id(SharedString::from(format!("host-{key}-{id}")))
            .group(group.clone())
            .relative()
            .h(px(HOST_ROW_H))
            .flex()
            .items_center()
            .gap_2()
            .pl_2()
            .pr_1()
            .rounded_sm()
            .cursor_pointer()
            .when_else(
                state.active,
                |d| d.bg(p.selected),
                |d| d.hover(|s| s.bg(p.raised)),
            )
            // Keyboard focus is a third state: not selected, but where the
            // arrows have got to. Without it, arrowing moves nothing visible.
            .when(focused, |d| d.border_1().border_color(p.accent))
            .when(state.connecting, |d| d.opacity(0.6))
            .when(state.connected, |d| {
                // A rail rather than a dot: it reads at the row's edge and
                // never competes with the icon.
                d.child(
                    div()
                        .id(SharedString::from(format!("rail-{key}-{id}")))
                        .absolute()
                        .left_0()
                        .top_1()
                        .bottom_1()
                        .w(px(2.))
                        .rounded_full()
                        .bg(p.good)
                        .tooltip(move |window, cx| {
                            Tooltip::new(rail_tip.clone()).build(window, cx)
                        }),
                )
            })
            .child(Icon::new(icon).small().text_color(tone))
            .child(
                v_flex()
                    .min_w_0()
                    .flex_1()
                    .child(
                        div()
                            .text_xs()
                            .text_color(p.ink)
                            .truncate()
                            .child(host.name.clone()),
                    )
                    .child(
                        h_flex()
                            .id(SharedString::from(format!("subtitle-{key}-{id}")))
                            .min_w_0()
                            .gap_1()
                            .text_xs()
                            .text_color(p.ink_2)
                            // A jump host is an icon, not words: "via jump
                            // host" pushed the address out of a sidebar-width
                            // row. The tooltip names it.
                            .when(jump.is_some(), |d| {
                                d.child(
                                    Icon::new(IconName::Route)
                                        .xsmall()
                                        .flex_none()
                                        .text_color(p.ink_3),
                                )
                            })
                            .child(div().min_w_0().truncate().child(subtitle))
                            .tooltip(move |window, cx| {
                                Tooltip::new(subtitle_tip.clone()).build(window, cx)
                            }),
                    ),
            )
            // Shown at rest, hidden under the hover buttons.
            .when(favorite, |d| {
                d.child(
                    div()
                        .flex_none()
                        .group_hover(group.clone(), |s| s.opacity(0.))
                        .child(Icon::new(IconName::StarFill).xsmall().text_color(p.warn)),
                )
            })
            .when(!age.is_empty(), |d| {
                d.child(
                    div()
                        .flex_none()
                        .text_xs()
                        .text_color(p.ink_3)
                        .group_hover(group.clone(), |s| s.opacity(0.))
                        .child(age),
                )
            })
            .when(state.connecting, |d| {
                d.child(Spinner::new().xsmall().color(p.accent))
            })
            // HostRow.vue's hover buttons. Absolutely placed, so revealing
            // them never reflows the row; shown on hover and on the row the
            // arrow keys have reached. Clicks cannot land while they are
            // hidden: the pointer over them is what hovers the row.
            .child(
                h_flex()
                    .absolute()
                    .right_1()
                    .top_0()
                    .bottom_0()
                    .items_center()
                    .gap_0p5()
                    .pl_2()
                    .bg(cluster_bg)
                    .opacity(if focused { 1. } else { 0. })
                    .group_hover(group, |s| s.opacity(1.))
                    .child(
                        IconButton::new(
                            SharedString::from(format!("fav-{key}-{id}")),
                            IconName::Star,
                            if favorite {
                                "Remove from favourites"
                            } else {
                                "Add to favourites"
                            },
                        )
                        .tone(p.warn)
                        .active(favorite)
                        .on_click(move |_, _, cx| {
                            cx.stop_propagation();
                            m_fav.update(cx, |m, cx| m.set_favorite(id, !favorite, cx));
                        }),
                    )
                    .child(
                        IconButton::new(
                            SharedString::from(format!("edit-{key}-{id}")),
                            IconName::Pencil,
                            "Edit host",
                        )
                        .on_click(move |_, window, cx| {
                            cx.stop_propagation();
                            open_host_editor(
                                window,
                                cx,
                                m_edit.clone(),
                                EditorKind::for_host(kind),
                                Some(id),
                                None,
                            );
                        }),
                    )
                    .child(
                        IconButton::new(
                            SharedString::from(format!("delete-{key}-{id}")),
                            IconName::Trash,
                            "Delete host",
                        )
                        .tone(p.bad)
                        .on_click(move |_, window, cx| {
                            cx.stop_propagation();
                            confirm_delete_host(window, cx, m_del.clone(), id);
                        }),
                    ),
            )
            // Dragging a row onto a group header moves it there. Only in the
            // grouped view, the one with headers to drop on; a press that
            // does not move is still a click.
            .when(grouped, |d| {
                d.on_drag(drag, |drag, _, _, cx| cx.new(|_| drag.clone()))
            })
            .on_click(cx.listener(move |this, e: &ClickEvent, window, cx| {
                this.focus_handle.focus(window, cx);
                if e.click_count() >= 2 {
                    if !this.model.read(cx).row_state(id).connecting {
                        this.activate(id, cx);
                    }
                } else {
                    this.focused_host = Some(id);
                    cx.notify();
                }
            }))
            .context_menu(move |menu, _, cx| {
                Self::host_context_menu(&model, &sidebar, id, menu, cx)
            })
            .into_any_element()
    }

    fn render_group_header(
        &self,
        name: &str,
        count: usize,
        collapsed: bool,
        p: &Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let toggle_name = name.to_string();
        let label: SharedString = if name.is_empty() {
            "Ungrouped".into()
        } else {
            name.to_string().into()
        };
        let model = self.model.clone();
        let menu_name = name.to_string();
        let drop_name = name.to_string();
        let ring = p.accent;
        div()
            .id(SharedString::from(format!("group-{name}")))
            .h(px(22.))
            .mt_1()
            .flex()
            .items_center()
            .justify_between()
            .pl_2()
            .pr_2()
            .rounded_sm()
            .border_l_2()
            // A rest-visible hue per group, hashed from the name so it
            // survives renames and reorders of the groups around it.
            .border_color(p.tag[group_accent_index(name)])
            .cursor_pointer()
            .hover(|s| s.bg(p.raised))
            .drag_over::<DraggedHost>(move |s, _, _, _| s.border_1().border_color(ring))
            .on_drop(cx.listener(move |this, drag: &DraggedHost, window, cx| {
                let current = this.model.read(cx).host(drag.id).map(|h| h.group.clone());
                if current.as_deref() != Some(drop_name.as_str()) {
                    move_to_group(window, cx, &this.model, drag.id, &drop_name);
                }
            }))
            .child(
                h_flex()
                    .gap_1()
                    .text_xs()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(p.ink_2)
                    .child(
                        Icon::new(if collapsed {
                            IconName::Folder
                        } else {
                            IconName::FolderOpen
                        })
                        .xsmall(),
                    )
                    .child(label),
            )
            .child(div().text_xs().text_color(p.ink_3).child(count.to_string()))
            .on_click(cx.listener(move |this, _, _, cx| {
                this.model
                    .update(cx, |m, cx| m.toggle_group(&toggle_name, cx));
            }))
            .context_menu(move |menu, _, cx| Self::group_context_menu(&model, &menu_name, menu, cx))
            .into_any_element()
    }
}

impl Render for HostSidebar {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = *palette(cx);
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_millis() as i64);
        let model_entity = self.model.clone();
        let model = model_entity.read(cx);
        let view_mode = model.view_mode();
        let total = model.hosts().len();
        let loaded = model.hosts_loaded();
        let query = self.query.clone();
        let shown = filter_hosts(model.hosts(), &query).len();

        // Collect everything the rows need before borrowing `cx` mutably.
        struct RowData {
            host: HostSummary,
            state: HostRowState,
        }
        enum Item {
            FavHeader,
            Fav(RowData),
            Group {
                name: String,
                count: usize,
                collapsed: bool,
            },
            Host(RowData),
        }
        let (rows, favs) = Self::nav_rows(model, &query);
        let group_counts: std::collections::HashMap<String, usize> = match view_mode {
            ViewMode::Grouped => {
                let filtered = filter_hosts(model.hosts(), &query);
                let rest: Vec<_> = filtered.into_iter().filter(|h| !h.favorite).collect();
                group_hosts(&rest, model.custom_groups())
                    .into_iter()
                    .map(|g| (g.name, g.hosts.len()))
                    .collect()
            }
            ViewMode::Flat => Default::default(),
        };
        let collapsed: HashSet<String> = model.collapsed_groups();
        let mut items = Vec::new();
        if !favs.is_empty() {
            items.push(Item::FavHeader);
            items.extend(favs.iter().map(|h| {
                Item::Fav(RowData {
                    host: (*h).clone(),
                    state: model.row_state(h.id),
                })
            }));
        }
        for row in &rows {
            items.push(match row {
                Row::Group { name } => Item::Group {
                    name: name.clone(),
                    count: group_counts.get(name).copied().unwrap_or(0),
                    collapsed: collapsed.contains(name),
                },
                Row::Item { item, .. } => Item::Host(RowData {
                    host: (*item).clone(),
                    state: model.row_state(item.id),
                }),
            });
        }

        let body: AnyElement = if !loaded {
            EmptyState::loading("hosts-loading", "Loading hosts")
                .compact()
                .into_any_element()
        } else if total == 0 {
            let model = self.model.clone();
            EmptyState::empty(
                "hosts-empty",
                "No hosts yet",
                "Add a host",
                move |_, window, cx| {
                    open_host_editor(window, cx, model.clone(), EditorKind::Ssh, None, None)
                },
            )
            .icon(Icon::new(IconName::Server))
            .hint("Add a server, a MongoDB connection or a Redis connection")
            .compact()
            .into_any_element()
        } else if shown == 0 {
            let this = cx.entity().downgrade();
            EmptyState::filtered(
                "hosts-filtered",
                "No matching hosts",
                "Clear search",
                move |_, window, cx| {
                    if let Some(this) = this.upgrade() {
                        this.update(cx, |s, cx| s.clear_search(window, cx));
                    }
                },
            )
            .hint(format!("{total} hidden by the search"))
            .compact()
            .into_any_element()
        } else {
            let mut children = Vec::with_capacity(items.len());
            for item in items {
                children.push(match item {
                    Item::FavHeader => h_flex()
                        .h(px(22.))
                        .px_2()
                        .gap_1()
                        .text_xs()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(p.ink_3)
                        .child(Icon::new(IconName::Star).xsmall().text_color(p.warn))
                        .child("Favorites")
                        .into_any_element(),
                    Item::Fav(r) => self.render_row(&r.host, r.state, "fav", now_ms, &p, cx),
                    Item::Group {
                        name,
                        count,
                        collapsed,
                    } => self.render_group_header(&name, count, collapsed, &p, cx),
                    Item::Host(r) => self.render_row(&r.host, r.state, "row", now_ms, &p, cx),
                });
            }
            div()
                .id("host-list-rows")
                .size_full()
                .overflow_y_scroll()
                .track_scroll(&self.scroll)
                .px_1()
                .py_1()
                .children(children)
                .into_any_element()
        };

        let toggle_label = match view_mode {
            ViewMode::Grouped => "Show as a flat list",
            ViewMode::Flat => "Group by folder",
        };
        let toggle_icon = match view_mode {
            ViewMode::Grouped => IconName::List,
            ViewMode::Flat => IconName::LayoutGrid,
        };
        let m = self.model.clone();
        let m2 = self.model.clone();
        let m_export = self.model.clone();
        let m_menu = self.model.clone();

        v_flex()
            .size_full()
            .bg(p.surface)
            .child(
                PanelHeader::new("Hosts")
                    .action(
                        IconButton::new("toggle-view", toggle_icon, toggle_label)
                            .on_click(move |_, _, cx| m.update(cx, |m, cx| m.toggle_view_mode(cx))),
                    )
                    .action(self.import_button())
                    .action(
                        IconButton::new("export-hosts", IconName::Upload, "Export hosts")
                            .on_click(move |_, window, cx| export_hosts(window, cx, &m_export)),
                    )
                    .action(
                        IconButton::new(
                            "new-local-shell",
                            IconName::SquareTerminal,
                            "Open local shell",
                        )
                        .on_click(move |_, _, cx| {
                            m2.update(cx, |m, cx| {
                                m.open_local(None, cx);
                            });
                        }),
                    )
                    .action(self.add_button()),
            )
            .child(
                div().px_2().py_1().border_b_1().border_color(p.line).child(
                    Input::new(&self.search)
                        .small()
                        .prefix(Icon::new(IconName::Search).xsmall().text_color(p.ink_3))
                        .cleanable(true),
                ),
            )
            .child(
                div()
                    .id("host-list")
                    .flex_1()
                    .min_h_0()
                    .track_focus(&self.focus_handle)
                    .key_context("HostList")
                    .on_key_down(cx.listener(Self::on_key_down))
                    .child(body)
                    .context_menu(move |menu, _, cx| Self::empty_context_menu(&m_menu, menu, cx)),
            )
    }
}
