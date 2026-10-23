//! The window: title bar, host sidebar, tab strip, tab area and the right
//! panel. Port of `MainWindow.vue`'s layout, shortcuts and close flow.

use std::collections::HashMap;
use std::rc::Rc;

use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::menu::{DropdownMenu as _, PopupMenuItem};
use gpui_kit::component::spinner::Spinner;
use gpui_kit::component::{
    Icon, Sizable as _, TitleBar, h_flex, resizable::HandleEdge, resize_handle, v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    Anchor, AnyElement, AnyView, App, AppContext as _, Axis, Context, DragMoveEvent, Empty, Entity,
    FocusHandle, Focusable, FontWeight, InteractiveElement, IntoElement, KeyBinding, ParentElement,
    Render, SharedString, StatefulInteractiveElement, Styled, Subscription, Window, div, px,
};

use super::host_editor::open_host_editor;
use super::host_sidebar::HostSidebar;
use super::settings::open_settings;
use super::window_state::WindowStateSaver;
use crate::app::host_form::EditorKind;
use crate::app::tab_kind::{TabKind, has_right_panel};
use crate::app::tabs::SftpLink;
use crate::app::{AppEvent, AppModel, LocalShellChoice};
use crate::backend::TabContentFactory;
use crate::panels::PanelKind;
use crate::ui::dialogs::{ConfirmOptions, confirm};
use crate::ui::empty_state::EmptyState;
use crate::ui::icon_button::IconButton;
use crate::ui::theme::{self, palette};
use crate::ui::toast::{ToastKind, toast};

gpui_kit::actions!(
    termdrop,
    [
        NewLocalTab,
        CloseActiveTab,
        NextTab,
        PrevTab,
        OpenSettings,
        ToggleRightPanel
    ]
);

const CONTEXT: &str = "TermDrop";

/// The window's shortcuts, the same chords as the Vue app's `onKeyDown`.
/// Bound in the window's own key context, so an input that owns a chord
/// (Ctrl+W deleting a word, say) keeps it while it has focus.
pub fn bind_keys(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("ctrl-shift-t", NewLocalTab, Some(CONTEXT)),
        KeyBinding::new("ctrl-w", CloseActiveTab, Some(CONTEXT)),
        KeyBinding::new("ctrl-tab", NextTab, Some(CONTEXT)),
        KeyBinding::new("ctrl-pagedown", NextTab, Some(CONTEXT)),
        KeyBinding::new("ctrl-shift-tab", PrevTab, Some(CONTEXT)),
        KeyBinding::new("ctrl-pageup", PrevTab, Some(CONTEXT)),
        KeyBinding::new("ctrl-,", OpenSettings, Some(CONTEXT)),
        KeyBinding::new("ctrl-alt-b", ToggleRightPanel, Some(CONTEXT)),
    ]);
}

/// Drag payloads for the two resize edges. GPUI tells drags apart by type.
#[derive(Clone)]
struct ResizeSidebar;
#[derive(Clone)]
struct ResizeRightPanel;

impl Render for ResizeSidebar {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        Empty
    }
}

impl Render for ResizeRightPanel {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        Empty
    }
}

pub struct MainWindow {
    model: Entity<AppModel>,
    sidebar: Entity<HostSidebar>,
    /// Each open tab's content, built once when the tab opens and kept until
    /// it closes, so a terminal's state survives switching tabs (the Vue app
    /// used `v-show` for the same reason).
    contents: HashMap<String, AnyView>,
    factory: Rc<dyn TabContentFactory>,
    focus_handle: FocusHandle,
    /// Saves the window's bounds as they change, for the next launch.
    window_state: WindowStateSaver,
    _subscriptions: Vec<Subscription>,
}

impl Focusable for MainWindow {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl MainWindow {
    pub fn new(
        model: Entity<AppModel>,
        factory: Rc<dyn TabContentFactory>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let sidebar = cx.new(|cx| HostSidebar::new(model.clone(), window, cx));
        let window_state = WindowStateSaver::new(model.read(cx).store().clone());
        let subscriptions = vec![
            cx.observe(&model, |_, _, cx| cx.notify()),
            cx.subscribe_in(&model, window, Self::on_app_event),
            cx.observe_window_bounds(window, |this, window, cx| {
                this.window_state
                    .changed(window, cx, |this: &mut Self| &mut this.window_state)
            }),
            cx.on_app_quit(|this, _| {
                this.window_state.flush();
                async {}
            }),
        ];
        let focus_handle = cx.focus_handle();
        // Shortcuts dispatch from the focused element up; with nothing
        // focused at launch, Ctrl+Shift+T would reach no one.
        focus_handle.focus(window, cx);
        Self {
            model,
            sidebar,
            contents: HashMap::new(),
            factory,
            focus_handle,
            window_state,
            _subscriptions: subscriptions,
        }
    }

    fn on_app_event(
        &mut self,
        model: &Entity<AppModel>,
        event: &AppEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            AppEvent::TabOpened(id) => {
                if let Some(tab) = model.read(cx).tab(id).cloned() {
                    let view = self.factory.build(&tab, window, cx);
                    self.contents.insert(id.clone(), view);
                }
            }
            AppEvent::TabClosed {
                tab,
                last_of_kind_for_host,
                ..
            } => {
                self.factory.tab_closed(tab, *last_of_kind_for_host, cx);
                self.contents.remove(&tab.id);
            }
            AppEvent::ThemeChanged(choice) => theme::install(*choice, cx),
            AppEvent::Error(message) => toast(window, cx, message.clone(), ToastKind::Error),
            AppEvent::ActiveTabChanged(_)
            | AppEvent::HostsChanged
            | AppEvent::FontSizeChanged(_)
            | AppEvent::SettingChanged(_) => {}
        }
        cx.notify();
    }

    /// A VNC tab owns the keyboard entirely: the remote desktop needs Ctrl+W,
    /// Ctrl+Tab and the rest far more than this window does.
    fn keyboard_owned_by_tab(&self, cx: &App) -> bool {
        self.model
            .read(cx)
            .active_tab()
            .is_some_and(|t| t.kind == TabKind::Vnc)
    }

    /// Closing asks first, as the Vue app does, then routes by kind.
    pub fn close_tab_with_confirm(
        &mut self,
        id: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(tab) = self.model.read(cx).tab(id).cloned() else {
            return;
        };
        let model = self.model.clone();
        confirm(
            window,
            cx,
            ConfirmOptions::new("Close Session", format!("Close session \"{}\"?", tab.title))
                .confirm_label("Close")
                .danger(),
            move |yes, _, cx| {
                if yes {
                    model.update(cx, |m, cx| {
                        m.close_tab(&tab.id, cx);
                    });
                }
            },
        );
    }

    /// Save the width a drag left behind. Registered on the window and on
    /// the handle itself: the handle follows the edge it drags, so the drag
    /// usually ends with the pointer on the handle, whose band occludes the
    /// window and would otherwise swallow the drop.
    fn save_sidebar_width(&mut self, _: &ResizeSidebar, _: &mut Window, cx: &mut Context<Self>) {
        self.model.update(cx, |m, cx| {
            let w = m.prefs().sidebar_width;
            m.set_sidebar_width(w, true, cx);
        });
    }

    fn save_right_width(&mut self, _: &ResizeRightPanel, _: &mut Window, cx: &mut Context<Self>) {
        self.model.update(cx, |m, cx| {
            let w = m.prefs().sftp_width;
            m.set_sftp_width(w, true, cx);
        });
    }

    fn render_tab_strip(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let p = *palette(cx);
        let model = self.model.read(cx);
        let active_id = model.active_tab_id().map(str::to_string);
        let show_panel_toggle = has_right_panel(model.active_tab());
        let panel_open = model.right_panel_open();
        let tabs: Vec<_> = model.tabs().to_vec();
        let entity = cx.entity().downgrade();

        let mut strip = h_flex().flex_1().min_w_0().overflow_x_hidden();
        for tab in tabs {
            let active = active_id.as_deref() == Some(tab.id.as_str());
            let status: AnyElement = match tab.kind {
                TabKind::Mongodb => Icon::new(IconName::Database)
                    .xsmall()
                    .text_color(p.accent)
                    .into_any_element(),
                TabKind::Redis => Icon::new(IconName::Layers)
                    .xsmall()
                    .text_color(p.redis)
                    .into_any_element(),
                TabKind::Vnc => Icon::new(IconName::Monitor)
                    .xsmall()
                    .text_color(p.vnc)
                    .into_any_element(),
                TabKind::Ssh | TabKind::Local => div()
                    .size(px(8.))
                    .rounded_full()
                    .bg(if tab.connected { p.good } else { p.bad })
                    .into_any_element(),
            };
            let id_click = tab.id.clone();
            let id_close = tab.id.clone();
            let entity_close = entity.clone();
            strip = strip.child(
                h_flex()
                    .id(SharedString::from(format!("tab-{}", tab.id)))
                    .h_full()
                    .flex_none()
                    .gap_1p5()
                    .pl_3()
                    .pr_1()
                    .border_r_1()
                    .border_color(p.line)
                    .text_xs()
                    .cursor_pointer()
                    .when(active, |d| d.bg(p.active).text_color(p.ink))
                    .when(!active, |d| {
                        d.text_color(p.ink_2).hover(|s| s.text_color(p.ink))
                    })
                    .child(status)
                    .child(tab.title.clone())
                    // The close button is always rendered beside the spinner,
                    // never swapped for it: a tab stuck mid-connect must not
                    // also lose the way out of that state.
                    .when(tab.connecting, |d| {
                        d.child(Spinner::new().xsmall().color(p.accent))
                    })
                    .child(
                        IconButton::new(
                            SharedString::from(format!("close-{}", tab.id)),
                            IconName::X,
                            "Close tab",
                        )
                        .on_click(move |_, window, cx| {
                            cx.stop_propagation();
                            if let Some(this) = entity_close.upgrade() {
                                this.update(cx, |this, cx| {
                                    this.close_tab_with_confirm(&id_close, window, cx)
                                });
                            }
                        }),
                    )
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.model.update(cx, |m, cx| m.activate_tab(&id_click, cx));
                    })),
            );
        }

        let m_local = self.model.clone();
        let shells = model.local_shells().to_vec();
        let m_panel = self.model.clone();
        let model_settings = self.model.clone();
        h_flex()
            .h(px(32.))
            .flex_none()
            .bg(p.surface)
            .border_b_1()
            .border_color(p.line)
            .child(strip)
            .child(
                h_flex()
                    .flex_none()
                    .px_1()
                    .gap_0p5()
                    .child(new_local_button(m_local, shells))
                    .when(show_panel_toggle, |d| {
                        d.child(
                            IconButton::new(
                                "toggle-right-panel",
                                if panel_open {
                                    IconName::PanelRightClose
                                } else {
                                    IconName::PanelRightOpen
                                },
                                if panel_open {
                                    "Hide side panel"
                                } else {
                                    "Show side panel"
                                },
                            )
                            .active(panel_open)
                            .on_click(move |_, _, cx| {
                                m_panel.update(cx, |m, cx| m.toggle_right_panel(cx))
                            }),
                        )
                    })
                    .child(
                        IconButton::new("open-settings", IconName::Settings, "Settings").on_click(
                            move |_, window, cx| open_settings(window, cx, model_settings.clone()),
                        ),
                    ),
            )
    }

    fn render_tab_area(&self, cx: &mut Context<Self>) -> AnyElement {
        let p = *palette(cx);
        let model = self.model.read(cx);
        if let Some(id) = model.active_tab_id()
            && let Some(view) = self.contents.get(id)
        {
            return div()
                .size_full()
                .bg(p.canvas)
                .child(view.clone())
                .into_any_element();
        }
        // The app's front door. It must offer something to do.
        let no_hosts = model.hosts_loaded() && model.hosts().is_empty();
        let m = self.model.clone();
        div()
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .bg(p.canvas)
            .child(
                div().w(px(420.)).child(
                    EmptyState::empty(
                        "no-tab",
                        if no_hosts {
                            "No hosts yet"
                        } else {
                            "No connection open"
                        },
                        if no_hosts {
                            "Add your first host"
                        } else {
                            "Add a host"
                        },
                        move |_, window, cx| {
                            open_host_editor(window, cx, m.clone(), EditorKind::Ssh, None, None)
                        },
                    )
                    .icon(Icon::new(IconName::Terminal))
                    .hint(if no_hosts {
                        "Add a server, a MongoDB connection or a Redis connection to begin"
                    } else {
                        "Pick a host on the left, or add another. Ctrl+Shift+T opens a local shell"
                    }),
                ),
            )
            .into_any_element()
    }

    /// The strip of panel tabs, as `MainWindow.vue`'s `PANEL_TABS`, with the
    /// hide button at its end.
    fn render_panel_strip(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let p = *palette(cx);
        let m = self.model.clone();
        let current = self.model.read(cx).right_panel_tab();
        let mut strip = h_flex()
            .h(px(32.))
            .flex_none()
            .border_b_1()
            .border_color(p.line);
        for kind in PanelKind::ALL {
            let selected = kind == current;
            strip = strip.child(
                div()
                    .id(SharedString::from(format!("panel-tab-{}", kind.label())))
                    .relative()
                    .flex_1()
                    .h_full()
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_xs()
                    .font_weight(FontWeight::MEDIUM)
                    .cursor_pointer()
                    .when(selected, |d| d.text_color(p.accent))
                    .when(!selected, |d| {
                        d.text_color(p.ink_2).hover(|s| s.text_color(p.ink))
                    })
                    .child(kind.label())
                    .when(selected, |d| {
                        d.child(
                            div()
                                .absolute()
                                .bottom_0()
                                .left_2()
                                .right_2()
                                .h(px(2.))
                                .rounded_full()
                                .bg(p.accent),
                        )
                    })
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.model
                            .update(cx, |m, cx| m.set_right_panel_tab(kind, cx));
                    })),
            );
        }
        strip.child(
            div().flex_none().px_0p5().child(
                IconButton::new("hide-right-panel", IconName::X, "Hide side panel")
                    .on_click(move |_, _, cx| m.update(cx, |m, cx| m.toggle_right_panel(cx))),
            ),
        )
    }

    fn render_right_panel(&self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = *palette(cx);
        let tab = self.model.read(cx).active_tab().cloned();
        let kind = self.model.read(cx).right_panel_tab();
        let body: AnyElement = match tab {
            None => Empty.into_any_element(),
            // The SFTP panel waits for the tab's SFTP session, as the Vue
            // panel showed "Connecting SFTP..." until it had one.
            Some(tab) if kind == PanelKind::Sftp && tab.sftp.session_id().is_none() => {
                match tab.sftp.clone() {
                    SftpLink::Failed(error) => {
                        let factory = self.factory.clone();
                        let model = self.model.clone();
                        let tab_id = tab.id.clone();
                        EmptyState::error(
                            "sftp-failed",
                            "SFTP is not available",
                            "Retry",
                            move |_, window, cx| {
                                if let Some(tab) = model.read(cx).tab(&tab_id).cloned() {
                                    factory.retry_sftp(&tab, window, cx);
                                }
                            },
                        )
                        .icon(Icon::new(IconName::FolderOpen))
                        .hint(error)
                        .compact()
                        .into_any_element()
                    }
                    _ => EmptyState::loading("sftp-connecting", "Connecting SFTP...")
                        .compact()
                        .into_any_element(),
                }
            }
            Some(tab) => match self.factory.right_panel(&tab, kind, window, cx) {
                Some(view) => view.into_any_element(),
                None => div()
                    .p_3()
                    .text_xs()
                    .text_color(p.ink_2)
                    .child("This panel is not available here.")
                    .into_any_element(),
            },
        };
        v_flex()
            .size_full()
            .bg(p.canvas)
            .border_l_1()
            .border_color(p.line)
            .child(self.render_panel_strip(cx))
            .child(div().flex_1().min_h_0().overflow_hidden().child(body))
    }
}

/// The new-terminal button. With one shell (or detection not finished) it
/// opens the default directly; with several it is a menu of them, as the Vue
/// tab bar's button is. Ctrl+Shift+T always opens the default.
fn new_local_button(model: Entity<AppModel>, shells: Vec<LocalShellChoice>) -> AnyElement {
    const LABEL: &str = "New local terminal (Ctrl+Shift+T)";
    if shells.len() < 2 {
        return IconButton::new("new-local-terminal", IconName::SquareTerminal, LABEL)
            .on_click(move |_, _, cx| {
                model.update(cx, |m, cx| {
                    m.open_local(None, cx);
                });
            })
            .into_any_element();
    }
    Button::new("new-local-terminal")
        .ghost()
        .small()
        .icon(Icon::new(IconName::SquareTerminal))
        .tooltip(LABEL)
        .accessibility_label(LABEL)
        .dropdown_menu_with_anchor(Anchor::TopRight, move |mut menu, _, _| {
            for shell in &shells {
                let model = model.clone();
                let shell = shell.clone();
                menu = menu.item(PopupMenuItem::new(shell.label.clone()).on_click(
                    move |_, _, cx| {
                        model.update(cx, |m, cx| {
                            m.open_local_shell(&shell, cx);
                        });
                    },
                ));
            }
            menu
        })
        .into_any_element()
}

impl Render for MainWindow {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = *palette(cx);
        let model = self.model.read(cx);
        let sidebar_w = model.prefs().sidebar_width;
        let right_w = model.prefs().sftp_width;
        let show_right = has_right_panel(model.active_tab()) && model.right_panel_open();
        let title: SharedString = match model.active_tab() {
            Some(tab) => format!("{} — {}", tab.title, crate::PRODUCT_NAME).into(),
            None => crate::PRODUCT_NAME.into(),
        };
        window.set_window_title(&title);
        let ui_font = theme::ui_font(cx);

        let floating = self.factory.floating();
        v_flex()
            .id("main-window")
            .relative()
            .size_full()
            .bg(p.canvas)
            .text_color(p.ink)
            .font_family(ui_font)
            .key_context(CONTEXT)
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(|this, _: &NewLocalTab, _, cx| {
                if this.keyboard_owned_by_tab(cx) {
                    cx.propagate();
                    return;
                }
                this.model.update(cx, |m, cx| {
                    m.open_local(None, cx);
                });
            }))
            .on_action(cx.listener(|this, _: &CloseActiveTab, window, cx| {
                if this.keyboard_owned_by_tab(cx) {
                    cx.propagate();
                    return;
                }
                if let Some(id) = this.model.read(cx).active_tab_id().map(str::to_string) {
                    this.close_tab_with_confirm(&id, window, cx);
                }
            }))
            .on_action(cx.listener(|this, _: &NextTab, _, cx| {
                if this.keyboard_owned_by_tab(cx) {
                    cx.propagate();
                    return;
                }
                this.model.update(cx, |m, cx| m.cycle_tabs(true, cx));
            }))
            .on_action(cx.listener(|this, _: &PrevTab, _, cx| {
                if this.keyboard_owned_by_tab(cx) {
                    cx.propagate();
                    return;
                }
                this.model.update(cx, |m, cx| m.cycle_tabs(false, cx));
            }))
            .on_action(cx.listener(|this, _: &OpenSettings, window, cx| {
                open_settings(window, cx, this.model.clone());
            }))
            .on_action(cx.listener(|this, _: &ToggleRightPanel, _, cx| {
                this.model.update(cx, |m, cx| m.toggle_right_panel(cx));
            }))
            // Widths follow the pointer while a drag is live and are saved
            // once, when it ends: a width changes on every move and the store
            // needs only the last one.
            .on_drag_move(
                cx.listener(|this, e: &DragMoveEvent<ResizeSidebar>, _, cx| {
                    let width = f32::from(e.event.position.x - e.bounds.left());
                    this.model
                        .update(cx, |m, cx| m.set_sidebar_width(width, false, cx));
                }),
            )
            .on_drag_move(
                cx.listener(|this, e: &DragMoveEvent<ResizeRightPanel>, _, cx| {
                    let width = f32::from(e.bounds.right() - e.event.position.x);
                    this.model
                        .update(cx, |m, cx| m.set_sftp_width(width, false, cx));
                }),
            )
            .on_drop(cx.listener(Self::save_sidebar_width))
            .on_drop(cx.listener(Self::save_right_width))
            .child(
                TitleBar::new().child(
                    h_flex()
                        .flex_1()
                        .gap_2()
                        .text_xs()
                        .child(
                            Icon::new(IconName::SquareTerminal)
                                .small()
                                .text_color(p.accent),
                        )
                        .child(
                            div()
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(p.ink)
                                .child(title),
                        ),
                ),
            )
            .child(
                h_flex()
                    .flex_1()
                    .min_h_0()
                    .child(
                        div()
                            .relative()
                            .h_full()
                            .flex_none()
                            .w(px(sidebar_w))
                            .child(self.sidebar.clone())
                            .child(
                                resize_handle::<ResizeSidebar, ResizeSidebar>(
                                    "sidebar-edge",
                                    Axis::Horizontal,
                                )
                                .inside(HandleEdge::Trailing)
                                .on_drag(ResizeSidebar, |drag, _, _, cx| cx.new(|_| drag.clone()))
                                .on_drop(cx.listener(Self::save_sidebar_width)),
                            ),
                    )
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            .h_full()
                            .child(self.render_tab_strip(cx))
                            .child(div().flex_1().min_h_0().child(self.render_tab_area(cx))),
                    )
                    .when(show_right, |row| {
                        row.child(
                            div()
                                .relative()
                                .h_full()
                                .flex_none()
                                .w(px(right_w))
                                .child(self.render_right_panel(window, cx))
                                .child(
                                    resize_handle::<ResizeRightPanel, ResizeRightPanel>(
                                        "right-panel-edge",
                                        Axis::Horizontal,
                                    )
                                    .inside(HandleEdge::Leading)
                                    .on_drag(ResizeRightPanel, |drag, _, _, cx| {
                                        cx.new(|_| drag.clone())
                                    })
                                    .on_drop(cx.listener(Self::save_right_width)),
                                ),
                        )
                    }),
            )
            // Panels' floating views (the SFTP editor): over the content,
            // under the dialog and notification layers, which the root draws
            // after this view. The layer itself takes no input.
            .child(
                div()
                    .absolute()
                    .top_0()
                    .left_0()
                    .size_full()
                    .children(floating),
            )
    }
}
