//! The system panel the status bar opens above itself: processes, network
//! and disk, from `get_system_panel` every 3 s while it is open and in
//! sight. The port of `TerminalTab.vue`'s "Expanded System Panel" and
//! `fetchPanelData` / `startSysPolling` in `useHostStatusPolling.js`.
//!
//! As in Vue, each time polling starts the panel shows a spinner until the
//! first answer, and the disk tab is filled from that first answer only
//! (later polls refresh processes and network).

use std::sync::Arc;
use std::time::Duration;

use gpui_kit::component::spinner::Spinner;
use gpui_kit::component::{h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, App, AppContext as _, AsyncApp, Context, Div, Entity, FontWeight, Hsla,
    InteractiveElement as _, IntoElement, ParentElement, Render, SharedString,
    StatefulInteractiveElement as _, Styled, Task, WeakEntity, Window, div, px, relative,
};
use termdrop_core::system::{DiskInfo, NetInterface, NetPort, NetworkInfo, Process};

use super::single_flight::{SingleFlight, joined};
use crate::core_backend::Services;
use crate::panels::common::{format_bytes, text_2xs};
use crate::ui::theme::{Palette, palette};

/// `PANEL_POLL_MS`.
const PANEL_POLL: Duration = Duration::from_millis(3000);
/// The panel's height (`h-48`).
const PANEL_HEIGHT: f32 = 192.;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SysTab {
    Processes,
    Network,
    Disk,
}

impl SysTab {
    const ALL: [SysTab; 3] = [SysTab::Processes, SysTab::Network, SysTab::Disk];

    /// The Vue labels are lower-case ids drawn `capitalize`d.
    fn label(self) -> &'static str {
        match self {
            SysTab::Processes => "Processes",
            SysTab::Network => "Network",
            SysTab::Disk => "Disk",
        }
    }
}

/// `visiblePorts`: the first eight.
pub(crate) fn visible_ports(network: Option<&NetworkInfo>) -> &[NetPort] {
    network.map_or(&[], |n| &n.ports[..n.ports.len().min(8)])
}

/// `visibleInterfaces`: the first four that are not loopback.
pub(crate) fn visible_interfaces(network: Option<&NetworkInfo>) -> Vec<&NetInterface> {
    network.map_or_else(Vec::new, |n| {
        n.interfaces
            .iter()
            .filter(|i| i.name != "lo")
            .take(4)
            .collect()
    })
}

/// `parseInt(s)` for a percent column; NaN (no digits) compares false, so
/// it is `None` here.
fn leading_int(s: &str) -> Option<i64> {
    let s = s.trim_start();
    let end = s
        .char_indices()
        .find(|&(i, c)| !(c.is_ascii_digit() || (i == 0 && (c == '-' || c == '+'))))
        .map_or(s.len(), |(i, _)| i);
    s[..end].parse().ok()
}

/// A mount's use colour: over 80 bad, over 60 warn, else good.
pub(crate) fn percent_tone(percent: &str, p: &Palette) -> Hsla {
    match leading_int(percent) {
        Some(n) if n > 80 => p.bad,
        Some(n) if n > 60 => p.warn,
        _ => p.good,
    }
}

/// `parseFloat(p.cpu) > 50`.
pub(crate) fn cpu_is_high(cpu: &str) -> bool {
    let s = cpu.trim_start();
    let end = s
        .char_indices()
        .find(|&(i, c)| !(c.is_ascii_digit() || c == '.' || (i == 0 && c == '-')))
        .map_or(s.len(), |(i, _)| i);
    s[..end].parse::<f64>().is_ok_and(|v| v > 50.0)
}

pub struct SystemPanel {
    services: Arc<Services>,
    host_id: i64,
    tab: SysTab,
    loading: bool,
    processes: Vec<Process>,
    network: Option<NetworkInfo>,
    disk: Option<DiskInfo>,
    poll: Option<Task<()>>,
    /// The `get_system_panel` call in flight, which a restarted poll awaits
    /// rather than sending another (see `single_flight`).
    fetch: SingleFlight<Result<termdrop_core::system::SystemPanel, String>>,
}

impl SystemPanel {
    pub(crate) fn new(services: Arc<Services>, host_id: i64, cx: &mut App) -> Entity<Self> {
        cx.new(|_| Self {
            services,
            host_id,
            tab: SysTab::Processes,
            loading: false,
            processes: Vec::new(),
            network: None,
            disk: None,
            poll: None,
            fetch: SingleFlight::new(),
        })
    }

    /// `startSysPolling` / `stopSysPolling`.
    pub(crate) fn set_polling(&mut self, on: bool, cx: &mut Context<Self>) {
        if !on {
            if self.poll.take().is_some() {
                self.loading = false;
                cx.notify();
            }
            return;
        }
        if self.poll.is_some() {
            return;
        }
        self.loading = true;
        cx.notify();
        self.poll = Some(
            cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
                let mut first = true;
                loop {
                    let Ok(job) = this.update(cx, |this, _| {
                        let services = this.services.clone();
                        let host_id = this.host_id;
                        this.fetch.get_or_start(move || {
                            let core = services.core.clone();
                            joined(
                                services
                                    .rt
                                    .spawn(async move { core.get_system_panel(host_id).await }),
                                "panel",
                            )
                        })
                    }) else {
                        return;
                    };
                    let result = job.await;
                    if this
                        .update(cx, |this, cx| this.loaded(result, first, cx))
                        .is_err()
                    {
                        return;
                    }
                    first = false;
                    cx.background_executor().timer(PANEL_POLL).await;
                }
            }),
        );
    }

    /// The tab's shell ended: a request still running speaks for the old
    /// session, so a reconnect does not wait on it.
    pub(crate) fn forget_inflight(&mut self) {
        self.fetch.forget();
    }

    fn loaded(
        &mut self,
        result: Result<termdrop_core::system::SystemPanel, String>,
        include_disk: bool,
        cx: &mut Context<Self>,
    ) {
        match result {
            Ok(panel) => {
                self.processes = panel.processes;
                self.network = Some(panel.network);
                if include_disk {
                    self.disk = Some(panel.disk);
                }
            }
            Err(e) => tracing::warn!("get_system_panel failed: {e}"),
        }
        self.loading = false;
        cx.notify();
    }

    // ---- rendering ---------------------------------------------------------

    /// A grid row of `span`s out of `of` columns, as the Vue `grid-cols-N`.
    fn cell(span: u32, of: u32) -> Div {
        div()
            .flex_none()
            .w(relative(span as f32 / of as f32))
            .min_w_0()
            .pr_1()
            .truncate()
    }

    fn heading(text: &'static str, p: &Palette) -> Div {
        div()
            .text_color(p.ink_3)
            .font_weight(FontWeight::MEDIUM)
            .mb(px(2.))
            .child(text)
    }

    fn row(id: SharedString, p: &Palette) -> gpui_kit::Stateful<Div> {
        let raised = p.raised;
        h_flex()
            .id(id)
            .py(px(2.))
            .text_color(p.ink)
            .hover(move |s| s.bg(raised))
    }

    fn render_processes(&self, p: &Palette) -> AnyElement {
        let right = |span| Self::cell(span, 12).flex().justify_end();
        v_flex()
            .child(
                h_flex()
                    .text_color(p.ink_3)
                    .font_weight(FontWeight::MEDIUM)
                    .border_b_1()
                    .border_color(p.line)
                    .pb(px(2.))
                    .mb(px(2.))
                    .child(Self::cell(1, 12).child("PID"))
                    .child(Self::cell(5, 12).child("Command"))
                    .child(right(2).child("CPU"))
                    .child(right(2).child("Mem"))
                    .child(right(2).child("Time")),
            )
            .children(self.processes.iter().enumerate().map(|(i, proc_)| {
                let hot = cpu_is_high(&proc_.cpu);
                Self::row(format!("proc-{i}-{}", proc_.pid).into(), p)
                    .child(
                        Self::cell(1, 12)
                            .font_family(crate::ui::theme::MONO_FONT)
                            .child(proc_.pid.clone()),
                    )
                    .child(Self::cell(5, 12).child(proc_.command.clone()))
                    .child(
                        right(2)
                            .when(hot, |d| d.text_color(p.bad))
                            .child(format!("{}%", proc_.cpu)),
                    )
                    .child(right(2).child(format!("{}%", proc_.mem)))
                    .child(right(2).text_color(p.ink_2).child(proc_.uptime.clone()))
            }))
            .into_any_element()
    }

    fn render_network(&self, p: &Palette) -> AnyElement {
        let network = self.network.as_ref();
        let ports = visible_ports(network);
        let interfaces = visible_interfaces(network);
        v_flex()
            .child(
                h_flex()
                    .mb_1()
                    .child(div().text_color(p.ink_2).child("Established:"))
                    .child(
                        div()
                            .text_color(p.ink)
                            .ml_1()
                            .child(network.map_or(0, |n| n.established_count).to_string()),
                    ),
            )
            .when(!ports.is_empty(), |col| {
                col.child(
                    v_flex()
                        .mb_1()
                        .child(Self::heading("Listening Ports", p))
                        .children(ports.iter().enumerate().map(|(i, port)| {
                            Self::row(format!("port-{i}").into(), p)
                                .child(Self::cell(1, 3).child(port.proto.clone()))
                                .child(Self::cell(1, 3).child(port.local.clone()))
                                .child(
                                    Self::cell(1, 3)
                                        .text_color(p.ink_2)
                                        .child(port.process.clone()),
                                )
                        })),
                )
            })
            .when(!interfaces.is_empty(), |col| {
                col.child(v_flex().child(Self::heading("Interfaces", p)).children(
                    interfaces.iter().map(|iface| {
                        h_flex()
                            .py(px(2.))
                            .text_color(p.ink)
                            .child(Self::cell(1, 4).child(iface.name.clone()))
                            .child(
                                Self::cell(1, 4)
                                    .text_color(p.ink_2)
                                    .child(format!("RX: {}", format_bytes(iface.rx_bytes))),
                            )
                            .child(
                                Self::cell(1, 4)
                                    .text_color(p.ink_2)
                                    .child(format!("TX: {}", format_bytes(iface.tx_bytes))),
                            )
                    }),
                ))
            })
            .into_any_element()
    }

    fn render_disk(&self, p: &Palette) -> AnyElement {
        let mounts = self.disk.as_ref().map_or(&[][..], |d| &d.mounts[..]);
        let dirs = self.disk.as_ref().map_or(&[][..], |d| &d.dirs[..]);
        v_flex()
            .when(!mounts.is_empty(), |col| {
                col.child(
                    v_flex()
                        .mb_1()
                        .child(Self::heading("Filesystems", p))
                        .children(mounts.iter().enumerate().map(|(i, m)| {
                            Self::row(format!("mount-{i}").into(), p)
                                .child(Self::cell(2, 6).child(m.mount.clone()))
                                .child(Self::cell(1, 6).child(m.size.clone()))
                                .child(Self::cell(1, 6).child(m.used.clone()))
                                .child(
                                    Self::cell(1, 6)
                                        .text_color(percent_tone(&m.percent, p))
                                        .child(format!("{}%", m.percent)),
                                )
                                .child(
                                    Self::cell(1, 6)
                                        .text_color(p.ink_2)
                                        .child(m.filesystem.clone()),
                                )
                        })),
                )
            })
            .when(!dirs.is_empty(), |col| {
                col.child(
                    v_flex()
                        .child(Self::heading("Top Directories", p).mt_1())
                        .children(dirs.iter().enumerate().map(|(i, d)| {
                            Self::row(format!("dir-{i}").into(), p)
                                .child(Self::cell(1, 2).child(d.path.clone()))
                                .child(Self::cell(1, 2).text_color(p.ink_2).child(d.size.clone()))
                        })),
                )
            })
            .into_any_element()
    }
}

impl Render for SystemPanel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = *palette(cx);
        let tabs = h_flex()
            .flex_none()
            .px_2()
            .border_b_1()
            .border_color(p.line)
            .children(SysTab::ALL.into_iter().map(|t| {
                let active = self.tab == t;
                let ink = p.ink;
                text_2xs(div())
                    .id(SharedString::from(format!("sys-tab-{}", t.label())))
                    .px_2()
                    .py(px(2.))
                    .font_weight(FontWeight::MEDIUM)
                    .cursor_pointer()
                    .text_color(if active { p.accent } else { p.ink_2 })
                    .when(!active, |d| d.hover(move |s| s.text_color(ink)))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.tab = t;
                        cx.notify();
                    }))
                    .child(t.label())
            }));
        let body = if self.loading {
            div()
                .size_full()
                .flex()
                .items_center()
                .justify_center()
                .child(Spinner::new().color(p.ink_2))
                .into_any_element()
        } else {
            match self.tab {
                SysTab::Processes => self.render_processes(&p),
                SysTab::Network => self.render_network(&p),
                SysTab::Disk => self.render_disk(&p),
            }
        };
        v_flex()
            .flex_none()
            .h(px(PANEL_HEIGHT))
            .border_t_1()
            .border_color(p.line)
            .bg(p.canvas)
            .child(tabs)
            .child(
                text_2xs(div())
                    .id("sys-panel-body")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .p_1()
                    .child(body),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn iface(name: &str) -> NetInterface {
        NetInterface {
            name: name.into(),
            rx_bytes: 0,
            tx_bytes: 0,
            rx_rate: 0,
            tx_rate: 0,
        }
    }

    fn port(n: u16) -> NetPort {
        NetPort {
            proto: "tcp".into(),
            local: format!("0.0.0.0:{n}"),
            state: "LISTEN".into(),
            process: String::new(),
        }
    }

    #[test]
    fn at_most_eight_ports_and_four_interfaces_without_loopback() {
        let network = NetworkInfo {
            ports: (0..12).map(port).collect(),
            interfaces: ["lo", "eth0", "eth1", "docker0", "br0", "veth1"]
                .into_iter()
                .map(iface)
                .collect(),
            established_count: 3,
        };
        assert_eq!(visible_ports(Some(&network)).len(), 8);
        let names: Vec<&str> = visible_interfaces(Some(&network))
            .into_iter()
            .map(|i| i.name.as_str())
            .collect();
        assert_eq!(names, vec!["eth0", "eth1", "docker0", "br0"]);
        assert!(visible_ports(None).is_empty());
        assert!(visible_interfaces(None).is_empty());
    }

    #[test]
    fn cpu_over_fifty_is_hot() {
        assert!(cpu_is_high("50.1"));
        assert!(!cpu_is_high("50"));
        assert!(!cpu_is_high("abc"));
        assert!(cpu_is_high("99.9%"));
    }

    #[test]
    fn mount_percent_tones() {
        assert_eq!(leading_int("81"), Some(81));
        assert_eq!(leading_int("61%"), Some(61));
        assert_eq!(leading_int("x"), None);
    }
}
