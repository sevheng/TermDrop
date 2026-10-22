//! The host list. Port of `HostSidebar.vue` + `HostRow.vue`: search, grouped
//! or flat view, collapsible groups, favourites, keyboard navigation and the
//! row context menu.
//!
//! The rules live in the pure modules (`host_grouping`, `list_nav`,
//! `host_row_state`); this file only lays them out. GPUI Kit's `Sidebar` is
//! not used for the list itself: its items are a menu model, and this list is
//! a keyboard-navigable tree with per-row state and context menus.

use std::collections::HashSet;

use gpui_kit::assets::IconName;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::menu::{ContextMenuExt as _, PopupMenuItem};
use gpui_kit::component::spinner::Spinner;
use gpui_kit::component::{Icon, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, App, AppContext as _, ClickEvent, Context, Entity, FocusHandle, Focusable,
    FontWeight, InteractiveElement, IntoElement, KeyDownEvent, ParentElement, Render, ScrollHandle,
    SharedString, StatefulInteractiveElement, Styled, Subscription, Window, div, px,
};

use crate::app::host_grouping::{filter_hosts, group_hosts};
use crate::app::host_kind::{HostKind, IconTone};
use crate::app::host_row_state::{HostRowState, format_age};
use crate::app::list_nav::{
    HOST_ROW_H, NavIntent, Row, dom_key, first_row, last_row, nav_intent, next_row, page_row,
    visible_rows,
};
use crate::app::prefs::ViewMode;
use crate::app::{AppEvent, AppModel};
use crate::backend::HostSummary;
use crate::ui::dialogs::{PromptOptions, prompt};
use crate::ui::empty_state::EmptyState;
use crate::ui::icon_button::IconButton;
use crate::ui::panel_header::PanelHeader;
use crate::ui::theme::{Palette, palette};

/// What the sidebar asks of the window, for actions it cannot finish alone.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SidebarEvent {
    /// The "Add a host" action. The host editor is a later phase, so the
    /// window decides what that means for now.
    AddHost,
}

impl gpui_kit::EventEmitter<SidebarEvent> for HostSidebar {}

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
            // Deleting a host needs the host editor's confirm path; the
            // other tree intents have nothing to act on while focus is only
            // ever on a host row.
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

    fn new_group(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let model = self.model.clone();
        prompt(
            window,
            cx,
            PromptOptions::new("New group")
                .placeholder("Group name")
                .confirm_label("Create"),
            move |name, _, cx| {
                if let Some(name) = name {
                    model.update(cx, |m, cx| m.add_custom_group(&name, cx));
                }
            },
        );
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
            HostKind::Ssh | HostKind::Local => IconName::Server,
        };
        let tone = match host.kind.icon_tone() {
            IconTone::Ink3 => p.ink_3,
            IconTone::Redis => p.redis,
            IconTone::Mongo => p.mongo,
            IconTone::Vnc => p.vnc,
        };
        let subtitle: SharedString = if host.host.is_empty() {
            host.address.clone().into()
        } else if host.username.is_empty() {
            host.host.clone().into()
        } else {
            format!("{}@{}", host.username, host.host).into()
        };
        let age = if state.connected {
            String::new()
        } else {
            format_age(host.last_connected_at.as_deref(), now_ms)
        };
        let verb = host.kind.activate_verb();
        let show_vnc = host.has_vnc && host.kind == HostKind::Ssh;
        let favorite = host.favorite;
        let model = self.model.clone();
        let sidebar = cx.entity().downgrade();

        div()
            .id(SharedString::from(format!("host-{key}-{id}")))
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
                        .absolute()
                        .left_0()
                        .top_1()
                        .bottom_1()
                        .w(px(2.))
                        .rounded_full()
                        .bg(p.good),
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
                        div()
                            .text_xs()
                            .text_color(p.ink_2)
                            .truncate()
                            .child(subtitle),
                    ),
            )
            .when(favorite, |d| {
                d.child(Icon::new(IconName::StarFill).xsmall().text_color(p.warn))
            })
            .when(!age.is_empty(), |d| {
                d.child(div().flex_none().text_xs().text_color(p.ink_3).child(age))
            })
            .when(state.connecting, |d| {
                d.child(Spinner::new().xsmall().color(p.accent))
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
            .context_menu(move |menu, _, _| {
                let m1 = model.clone();
                let m2 = model.clone();
                let m3 = model.clone();
                let m4 = model.clone();
                let sidebar = sidebar.clone();
                menu.item(
                    PopupMenuItem::new(verb)
                        .icon(IconName::Zap)
                        .on_click(move |_, _, cx| {
                            if let Some(s) = sidebar.upgrade() {
                                s.update(cx, |s, cx| s.activate(id, cx));
                            }
                        }),
                )
                .when(show_vnc, |menu| {
                    menu.item(
                        PopupMenuItem::new("Open remote desktop")
                            .icon(IconName::Monitor)
                            .on_click(move |_, _, cx| {
                                m1.update(cx, |m, cx| {
                                    m.open_vnc(id, cx);
                                });
                            }),
                    )
                })
                .item(
                    PopupMenuItem::new("Open local shell")
                        .icon(IconName::SquareTerminal)
                        .on_click(move |_, _, cx| {
                            m2.update(cx, |m, cx| {
                                m.open_local(None, cx);
                            });
                        }),
                )
                .separator()
                .item(
                    PopupMenuItem::new(if favorite { "Unfavorite" } else { "Favorite" })
                        .icon(IconName::Star)
                        .on_click(move |_, _, cx| {
                            m3.update(cx, |m, cx| m.set_favorite(id, !favorite, cx));
                        }),
                )
                .item(
                    PopupMenuItem::new("Copy name")
                        .icon(IconName::Copy)
                        .on_click(move |_, _, cx| {
                            if let Some(h) = m4.read(cx).host(id) {
                                cx.write_to_clipboard(gpui_kit::ClipboardItem::new_string(
                                    h.name.clone(),
                                ));
                            }
                        }),
                )
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
        let entity = cx.entity().downgrade();
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
            .border_color(p.line)
            .cursor_pointer()
            .hover(|s| s.bg(p.raised))
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
            .context_menu(move |menu, _, _| {
                let entity = entity.clone();
                menu.item(
                    PopupMenuItem::new("New group…")
                        .icon(IconName::FolderPlus)
                        .on_click(move |_, window, cx| {
                            if let Some(e) = entity.upgrade() {
                                e.update(cx, |s, cx| s.new_group(window, cx));
                            }
                        }),
                )
            })
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
            let this = cx.entity().downgrade();
            EmptyState::empty(
                "hosts-empty",
                "No hosts yet",
                "Add a host",
                move |_, _, cx| {
                    if let Some(this) = this.upgrade() {
                        this.update(cx, |_, cx| cx.emit(SidebarEvent::AddHost));
                    }
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
        let entity = cx.entity().downgrade();

        v_flex()
            .size_full()
            .bg(p.surface)
            .child(
                PanelHeader::new("Hosts")
                    .action(
                        IconButton::new("toggle-view", toggle_icon, toggle_label)
                            .on_click(move |_, _, cx| m.update(cx, |m, cx| m.toggle_view_mode(cx))),
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
                    ),
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
                    .context_menu(move |menu, _, _| {
                        let entity = entity.clone();
                        menu.item(
                            PopupMenuItem::new("New group…")
                                .icon(IconName::FolderPlus)
                                .on_click(move |_, window, cx| {
                                    if let Some(e) = entity.upgrade() {
                                        e.update(cx, |s, cx| s.new_group(window, cx));
                                    }
                                }),
                        )
                    }),
            )
    }
}
