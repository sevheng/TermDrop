//! The Docker panel (`DockerPanel.vue`, plus the logs/exec pane that
//! `TerminalTab.vue` hosts). Per host. See `../README.md`.
//!
//! Every command goes over the host's shared exec session, one at a time,
//! through the core (`docker_ps` is cached per host there). The list polls
//! every 5 s only while the panel is on screen: a hidden poll would hold the
//! exec session for nothing, which is why the Vue panel stopped on
//! deactivation.
//!
//! Only this folder is the Docker agent's to edit.

mod logic;
mod pane;

use std::time::Duration;

use gpui_kit::assets::IconName;
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::{ActiveTheme as _, Icon, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, AnyView, App, AppContext as _, Context, Entity, InteractiveElement as _,
    IntoElement, ParentElement as _, Render, SharedString, StatefulInteractiveElement as _,
    Styled as _, Task, Window, div, px,
};
use termdrop_core::docker::Container;

use self::logic::{
    ADD_TO_GROUP_COMMAND, Action, INSTALL_COMMAND, ListView, POLL_SECS, PaneKind, Problem,
    START_DAEMON_COMMAND, action_error_message, count_label, exec_shell, list_view, problem_of,
    status_line,
};
use self::pane::DockerPane;
use super::PanelContext;
use super::common::{list_row, text_2xs};
use crate::app::AppModel;
use crate::ui::dialogs::{ConfirmOptions, confirm};
use crate::ui::empty_state::EmptyState;
use crate::ui::icon_button::IconButton;
use crate::ui::panel_header::PanelHeader;
use crate::ui::theme::{Palette, palette};
use crate::ui::toast::{ToastKind, toast};

/// The panel's view, built the first time the Docker tab is shown for a host.
pub fn build(ctx: PanelContext, _window: &mut Window, cx: &mut App) -> AnyView {
    cx.new(|cx| DockerPanel::new(ctx, cx)).into()
}

struct DockerPanel {
    ctx: PanelContext,
    containers: Vec<Container>,
    /// A visible load (the spinner); polls and post-action reloads are silent.
    loading: bool,
    show_all: bool,
    problem: Option<Problem>,
    installing: bool,
    /// Bumped by every load, so only the latest one's answer is applied.
    load_seq: u64,
    load_task: Option<Task<()>>,
    poll_task: Option<Task<()>>,
    install_task: Option<Task<()>>,
    exec_task: Option<Task<()>>,
    was_shown: bool,
    _observe: gpui_kit::Subscription,
}

impl DockerPanel {
    fn new(ctx: PanelContext, cx: &mut Context<Self>) -> Self {
        let observe = cx.observe(&ctx.model, Self::on_model_changed);
        let mut this = Self {
            ctx,
            containers: Vec::new(),
            loading: false,
            show_all: false,
            problem: None,
            installing: false,
            load_seq: 0,
            load_task: None,
            poll_task: None,
            install_task: None,
            exec_task: None,
            // Built because it is being shown (`onMounted`).
            was_shown: true,
            _observe: observe,
        };
        this.load(false, cx);
        this.start_polling(cx);
        this
    }

    /// Shown again: a silent reload and the poll (`onActivated`). Hidden:
    /// the poll stops (`onDeactivated`).
    fn on_model_changed(&mut self, _: Entity<AppModel>, cx: &mut Context<Self>) {
        let shown = self.ctx.is_shown(cx);
        if shown == self.was_shown {
            return;
        }
        self.was_shown = shown;
        if shown {
            self.load(true, cx);
            self.start_polling(cx);
        } else {
            self.poll_task = None;
        }
    }

    fn start_polling(&mut self, cx: &mut Context<Self>) {
        self.poll_task = Some(cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_secs(POLL_SECS))
                    .await;
                let alive = this.update(cx, |this, cx| {
                    // A load already running answers for this tick.
                    if this.load_task.is_none() {
                        this.load(true, cx);
                    }
                });
                if alive.is_err() {
                    break;
                }
            }
        }));
    }

    /// `loadContainers(silent)`.
    fn load(&mut self, silent: bool, cx: &mut Context<Self>) {
        if !silent {
            self.loading = true;
            cx.notify();
        }
        self.load_seq += 1;
        let seq = self.load_seq;
        let core = self.ctx.core().clone();
        let host_id = self.ctx.host_id;
        let all = self.show_all;
        let job = self
            .ctx
            .spawn(async move { core.docker_ps(host_id, all).await });
        self.load_task = Some(cx.spawn(async move |this, cx| {
            let result = job.await.unwrap_or_else(|e| Err(e.to_string()));
            let _ = this.update(cx, |this, cx| this.loaded(seq, result, cx));
        }));
    }

    fn loaded(&mut self, seq: u64, result: Result<Vec<Container>, String>, cx: &mut Context<Self>) {
        if seq != self.load_seq {
            return;
        }
        self.load_task = None;
        self.loading = false;
        match result {
            Ok(containers) => {
                self.containers = containers;
                self.problem = None;
            }
            Err(e) => {
                self.problem = problem_of(&e);
                if self.problem.is_none() {
                    tracing::error!("docker_ps failed: {e}");
                }
                self.containers.clear();
            }
        }
        cx.notify();
    }

    fn set_show_all(&mut self, show_all: bool, cx: &mut Context<Self>) {
        self.show_all = show_all;
        self.load(false, cx);
    }

    /// This pipes a script from the internet into a root shell on the remote
    /// host, so it needs explicit consent rather than a single click.
    fn ask_install(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.installing {
            return;
        }
        let this = cx.entity().downgrade();
        confirm(
            window,
            cx,
            ConfirmOptions::new(
                "Install Docker",
                format!(
                    "This runs \"{INSTALL_COMMAND}\" on the remote host, which downloads and \
                     executes an installation script with root privileges. Continue?"
                ),
            )
            .danger(),
            move |yes, window, cx| {
                if yes {
                    let _ = this.update(cx, |this, cx| this.install(window, cx));
                }
            },
        );
    }

    fn install(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.installing = true;
        cx.notify();
        let core = self.ctx.core().clone();
        let host_id = self.ctx.host_id;
        let job = self
            .ctx
            .spawn(async move { core.docker_install(host_id).await });
        self.install_task = Some(cx.spawn_in(window, async move |this, cx| {
            let result = job.await.unwrap_or_else(|e| Err(e.to_string()));
            let _ = cx.update(|window, cx| {
                this.update(cx, |this, cx| {
                    this.installing = false;
                    match result {
                        Ok(_) => {
                            toast(
                                window,
                                cx,
                                "Docker installed successfully",
                                ToastKind::Success,
                            );
                            this.problem = None;
                            this.load(false, cx);
                        }
                        Err(e) => {
                            tracing::error!("docker_install failed: {e}");
                            toast(
                                window,
                                cx,
                                format!("Docker install failed: {e}"),
                                ToastKind::Error,
                            );
                        }
                    }
                    cx.notify();
                })
            });
        }));
    }

    /// Start runs at once; Stop and Restart ask first, as in Vue.
    fn ask_action(
        &mut self,
        action: Action,
        container: &Container,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let id = container.id.clone();
        match action.confirmation(&container.name) {
            None => self.run_action(action, id, window, cx),
            Some((title, message)) => {
                let this = cx.entity().downgrade();
                confirm(
                    window,
                    cx,
                    ConfirmOptions::new(title, message).danger(),
                    move |yes, window, cx| {
                        if yes {
                            let id = id.clone();
                            let _ =
                                this.update(cx, |this, cx| this.run_action(action, id, window, cx));
                        }
                    },
                );
            }
        }
    }

    /// The optimistic row update, the command, then a silent reload either
    /// way. `container_id` came from `docker_ps`, which is what makes it safe
    /// to put into the remote command unquoted.
    fn run_action(
        &mut self,
        action: Action,
        container_id: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(c) = self.containers.iter_mut().find(|c| c.id == container_id) {
            action.apply(c);
            cx.notify();
        }
        let core = self.ctx.core().clone();
        let host_id = self.ctx.host_id;
        let job = self.ctx.spawn(async move {
            match action {
                Action::Start => core.docker_start(host_id, container_id).await,
                Action::Stop => core.docker_stop(host_id, container_id).await,
                Action::Restart => core.docker_restart(host_id, container_id).await,
            }
        });
        cx.spawn_in(window, async move |this, cx| {
            let result = job.await.unwrap_or_else(|e| Err(e.to_string()));
            let _ = cx.update(|window, cx| {
                this.update(cx, |this, cx| {
                    if let Err(e) = result {
                        toast(
                            window,
                            cx,
                            action_error_message(&e, action.verb()),
                            ToastKind::Error,
                        );
                    }
                    this.load(true, cx);
                })
            });
        })
        .detach();
    }

    /// `viewLogs`: follow a running container, show the tail of a stopped
    /// one.
    fn view_logs(&mut self, container: &Container, window: &mut Window, cx: &mut Context<Self>) {
        self.open_pane(
            PaneKind::Logs,
            container.name.clone(),
            String::new(),
            container.running,
            window,
            cx,
        );
    }

    /// `execInto`: ask the container which shell it has, then open the pane.
    fn exec_into(&mut self, container: &Container, window: &mut Window, cx: &mut Context<Self>) {
        let core = self.ctx.core().clone();
        let host_id = self.ctx.host_id;
        let id = container.id.clone();
        let name = container.name.clone();
        let job = self
            .ctx
            .spawn(async move { core.docker_inspect_shell(host_id, id).await });
        self.exec_task = Some(cx.spawn_in(window, async move |this, cx| {
            let result = job.await.unwrap_or_else(|e| Err(e.to_string()));
            let _ = cx.update(|window, cx| {
                this.update(cx, |this, cx| match result {
                    Ok(shell) => {
                        let shell = exec_shell(&shell).to_string();
                        this.open_pane(PaneKind::Exec, name, shell, false, window, cx);
                    }
                    Err(e) => toast(
                        window,
                        cx,
                        action_error_message(&e, "Exec"),
                        ToastKind::Error,
                    ),
                })
            });
        }));
    }

    fn open_pane(
        &mut self,
        kind: PaneKind,
        name: String,
        shell: String,
        follow: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let pane = DockerPane::new(self.ctx.clone(), kind, name, shell, cx);
        // The tab owns the pane from here; the panel keeps nothing.
        match self.ctx.show_terminal_pane(pane.clone().into(), cx) {
            Some(tab_id) => pane.update(cx, |p, cx| p.shown_in(tab_id, follow, window, cx)),
            None => toast(
                window,
                cx,
                "Open a terminal on this host to show the pane",
                ToastKind::Warning,
            ),
        }
    }

    // ---- rendering --------------------------------------------------------

    fn render_body(&self, p: &Palette, cx: &mut Context<Self>) -> AnyElement {
        match list_view(
            self.loading,
            self.problem,
            self.containers.len(),
            self.show_all,
        ) {
            ListView::Loading => {
                EmptyState::loading("docker-loading", "Loading containers\u{2026}")
                    .into_any_element()
            }
            ListView::Problem(Problem::NotInstalled) => {
                let state = if self.installing {
                    EmptyState::loading(
                        "docker-installing",
                        "Installing\u{2026} this may take a minute",
                    )
                } else {
                    EmptyState::empty(
                        "docker-install",
                        "Docker is not installed",
                        "Install Docker",
                        cx.listener(|this, _, window, cx| this.ask_install(window, cx)),
                    )
                    .icon(Icon::new(IconName::Container))
                    .hint("This host does not have Docker available")
                };
                v_flex()
                    .items_center()
                    .child(state)
                    .child(
                        text_2xs(
                            div()
                                .font_family(cx.theme().mono_font_family.clone())
                                .text_color(p.ink_3)
                                .mt_2(),
                        )
                        .child(INSTALL_COMMAND),
                    )
                    .into_any_element()
            }
            ListView::Problem(Problem::DaemonNotRunning) => self.problem_state(
                "docker-daemon",
                "Docker daemon is not running",
                "Docker is installed but the service is stopped",
                "Start it by running in a terminal:",
                START_DAEMON_COMMAND,
                None,
                p,
                cx,
            ),
            ListView::Problem(Problem::PermissionDenied) => self.problem_state(
                "docker-permission",
                "Docker permission denied",
                "Your user is not in the docker group",
                "Fix by running in a terminal:",
                ADD_TO_GROUP_COMMAND,
                Some("Then reconnect this session"),
                p,
                cx,
            ),
            // "None running" and "none at all" are different answers, and
            // the Show all toggle is what resolves the first.
            ListView::Empty { show_all: false } => EmptyState::filtered(
                "docker-none-running",
                "No running containers",
                "Show all",
                cx.listener(|this, _, _, cx| this.set_show_all(true, cx)),
            )
            .icon(Icon::new(IconName::Container))
            .hint("Stopped containers are hidden")
            .into_any_element(),
            ListView::Empty { show_all: true } => EmptyState::empty(
                "docker-none",
                "No containers on this host",
                "Refresh",
                cx.listener(|this, _, _, cx| this.load(false, cx)),
            )
            .icon(Icon::new(IconName::Container))
            .into_any_element(),
            ListView::List => div()
                .id("docker-containers")
                .size_full()
                .overflow_y_scroll()
                .children(
                    self.containers
                        .iter()
                        .enumerate()
                        .map(|(i, c)| self.render_row(i, c, p, cx)),
                )
                .into_any_element(),
        }
    }

    /// A problem the user fixes in a terminal: the command stays copyable
    /// text rather than being flattened into prose. Refresh re-checks.
    #[allow(clippy::too_many_arguments)]
    fn problem_state(
        &self,
        id: &'static str,
        title: &'static str,
        hint: &'static str,
        lead: &'static str,
        command: &'static str,
        after: Option<&'static str>,
        p: &Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        v_flex()
            .items_center()
            .child(
                EmptyState::empty(
                    id,
                    title,
                    "Refresh",
                    cx.listener(|this, _, _, cx| this.load(false, cx)),
                )
                .icon(Icon::new(IconName::Container))
                .hint(hint),
            )
            .child(
                v_flex()
                    .max_w(px(320.))
                    .mt_3()
                    .px_3()
                    .py_2()
                    .gap_1()
                    .bg(p.surface)
                    .border_1()
                    .border_color(p.line)
                    .rounded_sm()
                    .child(text_2xs(div().text_color(p.ink_3)).child(lead))
                    .child(
                        text_2xs(
                            div()
                                .font_family(cx.theme().mono_font_family.clone())
                                .text_color(p.good),
                        )
                        .child(command),
                    )
                    .when_some(after, |col, after| {
                        col.child(text_2xs(div().text_color(p.ink_3)).child(after))
                    }),
            )
            .into_any_element()
    }

    fn render_row(
        &self,
        i: usize,
        c: &Container,
        p: &Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let button_id = |what: &str| SharedString::from(format!("docker-{what}-{i}"));
        let mut actions = h_flex().flex_none().gap_0p5();
        if c.running {
            let row = c.clone();
            actions = actions.child(
                IconButton::new(button_id("stop"), IconName::Square, "Stop").on_click(cx.listener(
                    move |this, _, window, cx| this.ask_action(Action::Stop, &row, window, cx),
                )),
            );
        } else {
            let row = c.clone();
            actions = actions.child(
                IconButton::new(button_id("start"), IconName::Play, "Start").on_click(cx.listener(
                    move |this, _, window, cx| this.ask_action(Action::Start, &row, window, cx),
                )),
            );
        }
        let restart = c.clone();
        let logs = c.clone();
        let exec = c.clone();
        actions = actions
            .child(
                IconButton::new(button_id("restart"), IconName::RotateCcw, "Restart").on_click(
                    cx.listener(move |this, _, window, cx| {
                        this.ask_action(Action::Restart, &restart, window, cx)
                    }),
                ),
            )
            .child(
                IconButton::new(button_id("logs"), IconName::FileText, "Logs").on_click(
                    cx.listener(move |this, _, window, cx| this.view_logs(&logs, window, cx)),
                ),
            )
            .child(
                IconButton::new(button_id("exec"), IconName::Terminal, "Exec").on_click(
                    cx.listener(move |this, _, window, cx| this.exec_into(&exec, window, cx)),
                ),
            );

        let dot = if c.running { p.good } else { p.ink_3 };
        list_row(
            h_flex()
                .id(SharedString::from(format!("docker-row-{i}")))
                .gap_2()
                .px_2()
                .py_1(),
            p,
            false,
            false,
        )
        .hover(|s| s.bg(p.raised))
        .child(div().flex_none().size_2().rounded_full().bg(dot))
        .child(
            v_flex()
                .flex_1()
                .min_w_0()
                .child(
                    div()
                        .text_xs()
                        .text_color(p.ink)
                        .truncate()
                        .child(c.name.clone()),
                )
                .child(text_2xs(div().text_color(p.ink_2).truncate()).child(c.image.clone()))
                .child(text_2xs(div().text_color(p.ink_3).truncate()).child(status_line(c))),
        )
        .child(actions)
        .into_any_element()
    }
}

impl Render for DockerPanel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = *palette(cx);
        let meta = count_label(self.containers.len());
        let header = PanelHeader::new("Docker")
            .when(!meta.is_empty(), |h| {
                h.action(text_2xs(div().text_color(p.ink_3).mr_1()).child(meta))
            })
            .action(
                Checkbox::new("docker-show-all")
                    .label("Show all")
                    .checked(self.show_all)
                    .on_click(
                        cx.listener(|this, checked: &bool, _, cx| this.set_show_all(*checked, cx)),
                    ),
            )
            .action(
                IconButton::new("docker-refresh", IconName::RefreshCw, "Refresh containers")
                    .disabled(self.loading)
                    .on_click(cx.listener(|this, _, _, cx| this.load(false, cx))),
            );
        v_flex().size_full().bg(p.canvas).child(header).child(
            div()
                .flex_1()
                .min_h(px(80.))
                .relative()
                .child(self.render_body(&p, cx)),
        )
    }
}
