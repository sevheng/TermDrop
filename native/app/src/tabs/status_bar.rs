//! The host status bar under an SSH terminal, and the system panel it opens:
//! the port of `TerminalTab.vue`'s status bar and
//! `composables/useHostStatusPolling.js`.
//!
//! - `get_system_stats` every 5 s, only while the tab is the active one, its
//!   shell is connected and the window can be seen. Polls run one after
//!   another (Vue's `setInterval` could overlap a slow one with the next).
//! - The last status and the network counters are kept per host for the
//!   life of the app (the Vue store's `systemStatus` / `prevNetStats` maps),
//!   so a tab shown again starts from what it last showed and its rates
//!   carry on. The first sample of a host shows the cumulative totals.
//! - A disconnect clears the bar to a pulsing "Disconnected" until the shell
//!   is back; the cache is left alone, as the store's was.
//! - Clicking the bar opens the system panel above it
//!   ([`super::system_panel`]).

use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use gpui_kit::assets::IconName;
use gpui_kit::component::spinner::Spinner;
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{Icon, Sizable as _, h_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    Animation, AnimationExt as _, AnyElement, App, AppContext as _, AsyncApp, Context, Entity,
    FontWeight, Hsla, InteractiveElement as _, IntoElement, ParentElement, Render, SharedString,
    StatefulInteractiveElement as _, Styled, Subscription, Task, WeakEntity, Window,
    WindowVisibility, div, pulsating_between, px,
};

use super::system_panel::SystemPanel;
use crate::app::{AppEvent, AppModel};
use crate::core_backend::Services;
use crate::panels::common::{format_bytes, format_rate, text_2xs};
use crate::ui::theme::{Palette, palette};

/// `STATUS_POLL_MS`.
const STATUS_POLL: Duration = Duration::from_millis(5000);

/// What the bar shows: `EMPTY_STATUS`'s fields. Every one is display text;
/// an empty one is not shown (the network pair shows a dash).
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct HostStatus {
    pub load: String,
    pub ram: String,
    pub disk: String,
    pub uptime: String,
    /// OS, kernel and architecture, joined with " · ".
    pub os: String,
    pub cores: String,
    pub net_down: String,
    pub net_up: String,
}

/// The network counters at the last poll (`prevNetStats`); `time_ms` 0
/// means none yet.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub(crate) struct NetSample {
    pub rx: i64,
    pub tx: i64,
    pub time_ms: u64,
}

#[derive(Default)]
struct Cached {
    status: Option<HostStatus>,
    net: NetSample,
}

thread_local! {
    /// Per host, for the life of the app. UI thread only.
    static CACHE: RefCell<HashMap<i64, Cached>> = RefCell::new(HashMap::new());
}

fn cached_status(host_id: i64) -> Option<HostStatus> {
    CACHE.with(|c| c.borrow().get(&host_id).and_then(|e| e.status.clone()))
}

fn cached_net(host_id: i64) -> NetSample {
    CACHE.with(|c| c.borrow().get(&host_id).map(|e| e.net).unwrap_or_default())
}

fn store(host_id: i64, status: HostStatus, net: Option<NetSample>) {
    CACHE.with(|c| {
        let mut c = c.borrow_mut();
        let entry = c.entry(host_id).or_default();
        entry.status = Some(status);
        if let Some(net) = net {
            entry.net = net;
        }
    });
}

/// JavaScript's `parseInt(s) || 0`: the leading integer, or 0.
fn parse_int_prefix(s: &str) -> i64 {
    let s = s.trim_start();
    let (sign, digits) = match s.strip_prefix('-') {
        Some(rest) => (-1, rest),
        None => (1, s.strip_prefix('+').unwrap_or(s)),
    };
    let end = digits
        .find(|c: char| !c.is_ascii_digit())
        .unwrap_or(digits.len());
    digits[..end].parse::<i64>().map(|n| sign * n).unwrap_or(0)
}

/// `computeNetTotals`: rx and tx summed over the `netdev` lines
/// ("iface: rx tx"), loopback left out.
pub(crate) fn compute_net_totals(netdev: &str) -> (i64, i64) {
    let mut rx = 0i64;
    let mut tx = 0i64;
    for line in netdev.split('\n') {
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() >= 3 {
            let iface = parts[0].replacen(':', "", 1);
            if iface == "lo" {
                continue;
            }
            rx = rx.saturating_add(parse_int_prefix(parts[1]));
            tx = tx.saturating_add(parse_int_prefix(parts[2]));
        }
    }
    (rx, tx)
}

/// One `get_system_stats` result made into the bar's text, given the
/// previous counters and the time now (ms since the epoch, `Date.now()`).
/// Returns the new counters to keep when the result had any.
pub(crate) fn status_from_stats(
    result: &serde_json::Value,
    prev: NetSample,
    now_ms: u64,
) -> (HostStatus, Option<NetSample>) {
    let field = |k: &str| result.get(k).and_then(|v| v.as_str()).unwrap_or("");
    let mut net_down = String::new();
    let mut net_up = String::new();
    let mut sample = None;
    let netdev = field("netdev");
    if !netdev.is_empty() {
        let (rx, tx) = compute_net_totals(netdev);
        if prev.time_ms > 0 && prev.rx > 0 && prev.tx > 0 {
            let elapsed = now_ms.saturating_sub(prev.time_ms) as f64 / 1000.0;
            if elapsed > 0.0 {
                net_down = format_rate((rx - prev.rx) as f64 / elapsed);
                net_up = format_rate((tx - prev.tx) as f64 / elapsed);
            }
        } else {
            // First sample: the totals so far, rather than a dash.
            net_down = format_bytes(rx.max(0) as u64);
            net_up = format_bytes(tx.max(0) as u64);
        }
        sample = Some(NetSample {
            rx,
            tx,
            time_ms: now_ms,
        });
    }
    let os = [field("os"), field("kernel"), field("arch")]
        .into_iter()
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join(" \u{b7} ");
    let status = HostStatus {
        load: field("load").to_string(),
        ram: field("ram").to_string(),
        disk: field("disk").to_string(),
        uptime: field("uptime").to_string(),
        os,
        cores: field("cores").to_string(),
        net_down,
        net_up,
    };
    (status, sample)
}

/// The error as the bar shows it: `String(err).replace(/^Error: /, '')`.
fn error_text(e: &str) -> String {
    e.strip_prefix("Error: ").unwrap_or(e).to_string()
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// The shell's state, as far as the bar cares.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Link {
    /// Connecting, the first time or again. A reconnect keeps showing
    /// "Disconnected" until it succeeds, as Vue's `isDisconnected` did.
    Connecting,
    Connected,
    Ended,
}

pub struct StatusBar {
    services: Arc<Services>,
    model: Entity<AppModel>,
    tab_id: String,
    host_id: i64,
    status: HostStatus,
    /// A poll is in flight. Starts true, so a tab still connecting reads
    /// "Loading stats…" rather than an empty bar.
    loading: bool,
    error: Option<String>,
    connected: bool,
    disconnected: bool,
    window_visible: bool,
    expanded: bool,
    panel: Option<Entity<SystemPanel>>,
    poll: Option<Task<()>>,
    _subscriptions: Vec<Subscription>,
}

impl StatusBar {
    pub(crate) fn new(
        services: Arc<Services>,
        model: Entity<AppModel>,
        tab_id: String,
        host_id: i64,
        window: &mut Window,
        cx: &mut App,
    ) -> Entity<Self> {
        cx.new(|cx| {
            let subscriptions = vec![
                cx.subscribe(&model, |this: &mut Self, _, event: &AppEvent, cx| {
                    if let AppEvent::ActiveTabChanged(_) = event {
                        this.update_polling(cx);
                    }
                }),
                // `visibilitychange`: a minimised window stops polling.
                cx.observe_window_visibility(window, |this, _: WindowVisibility, window, cx| {
                    this.window_visible = window.is_visible();
                    this.update_polling(cx)
                }),
            ];
            Self {
                services,
                model,
                tab_id,
                host_id,
                status: HostStatus::default(),
                loading: true,
                error: None,
                connected: false,
                disconnected: false,
                window_visible: window.is_visible(),
                expanded: false,
                panel: None,
                poll: None,
                _subscriptions: subscriptions,
            }
        })
    }

    /// The terminal's session changed state.
    pub(crate) fn set_link(&mut self, link: Link, cx: &mut Context<Self>) {
        match link {
            Link::Connecting => self.connected = false,
            Link::Connected => {
                self.connected = true;
                self.disconnected = false;
            }
            Link::Ended => {
                // `onDisconnected`: stop, and `resetStatus`.
                self.connected = false;
                self.disconnected = true;
                self.status = HostStatus::default();
                self.error = None;
            }
        }
        self.update_polling(cx);
        cx.notify();
    }

    fn is_active_tab(&self, cx: &App) -> bool {
        self.model.read(cx).active_tab_id() == Some(self.tab_id.as_str())
    }

    /// Start or stop both polls to match what can be seen.
    fn update_polling(&mut self, cx: &mut Context<Self>) {
        let on_screen = self.is_active_tab(cx) && self.window_visible;
        let poll = on_screen && self.connected;
        if poll && self.poll.is_none() {
            self.start_polling(cx);
        } else if !poll {
            self.poll = None;
        }
        // The panel polls only while it is open and in sight. Vue's kept
        // polling an open panel behind another tab; nothing could see it.
        let panel_on = on_screen && self.expanded;
        if let Some(panel) = &self.panel {
            panel.update(cx, |p, cx| p.set_polling(panel_on, cx));
        }
    }

    /// `startStatusPolling`: the cached status at once, then a poll now and
    /// every 5 s.
    fn start_polling(&mut self, cx: &mut Context<Self>) {
        if let Some(cached) = cached_status(self.host_id) {
            self.status = cached;
        }
        self.poll = Some(
            cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
                loop {
                    let Ok(job) = this.update(cx, |this, cx| this.begin_fetch(cx)) else {
                        return;
                    };
                    let result = job
                        .await
                        .unwrap_or_else(|e| Err(format!("stats task failed: {e}")));
                    if this
                        .update(cx, |this, cx| this.finish_fetch(result, cx))
                        .is_err()
                    {
                        return;
                    }
                    cx.background_executor().timer(STATUS_POLL).await;
                }
            }),
        );
    }

    fn begin_fetch(
        &mut self,
        cx: &mut Context<Self>,
    ) -> tokio::task::JoinHandle<Result<serde_json::Value, String>> {
        self.loading = true;
        cx.notify();
        let core = self.services.core.clone();
        let host_id = self.host_id;
        self.services
            .rt
            .spawn(async move { core.get_system_stats(host_id).await })
    }

    fn finish_fetch(&mut self, result: Result<serde_json::Value, String>, cx: &mut Context<Self>) {
        self.loading = false;
        // A poll that was in flight when the shell dropped says nothing
        // about the host now.
        if self.disconnected {
            cx.notify();
            return;
        }
        match result {
            Ok(value) => {
                let (status, net) = status_from_stats(&value, cached_net(self.host_id), now_ms());
                store(self.host_id, status.clone(), net);
                self.status = status;
                self.error = None;
            }
            Err(e) => {
                tracing::warn!("get_system_stats failed: {e}");
                self.error = Some(error_text(&e));
            }
        }
        cx.notify();
    }

    fn toggle(&mut self, cx: &mut Context<Self>) {
        self.expanded = !self.expanded;
        if self.expanded && self.panel.is_none() {
            let panel = SystemPanel::new(self.services.clone(), self.host_id, cx);
            self.panel = Some(panel);
        }
        self.update_polling(cx);
        cx.notify();
    }

    // ---- rendering ---------------------------------------------------------

    fn item(icon: IconName, tint: Hsla, p: &Palette) -> gpui_kit::Div {
        text_2xs(
            h_flex()
                .flex_none()
                .gap_1()
                .text_color(p.ink_2)
                .whitespace_nowrap(),
        )
        .child(Icon::new(icon).size(px(10.)).text_color(tint))
    }

    fn labelled(
        icon: IconName,
        tint: Hsla,
        label: &'static str,
        value: String,
        p: &Palette,
    ) -> AnyElement {
        Self::item(icon, tint, p)
            .child(div().font_weight(FontWeight::MEDIUM).child(label))
            .child(value)
            .into_any_element()
    }

    fn render_stats(&self, p: &Palette) -> Vec<AnyElement> {
        let s = &self.status;
        let mut out = Vec::new();
        if !s.os.is_empty() {
            let os: SharedString = s.os.clone().into();
            let tip = os.clone();
            out.push(
                Self::item(IconName::Monitor, p.accent, p)
                    .id("status-os")
                    .max_w(px(140.))
                    .min_w_0()
                    .flex_shrink(1.)
                    .cursor_default()
                    .child(div().min_w_0().truncate().child(os))
                    .tooltip(move |window, cx| Tooltip::new(tip.clone()).build(window, cx))
                    .into_any_element(),
            );
        }
        // Tints: Vue's syn-teal / syn-blue / syn-purple have no token in the
        // native palette yet; accent stands in for all three.
        if !s.load.is_empty() {
            let value = if s.cores.is_empty() {
                s.load.clone()
            } else {
                format!("{} / {} cores", s.load, s.cores)
            };
            out.push(Self::labelled(IconName::Cpu, p.accent, "CPU:", value, p));
        }
        if !s.ram.is_empty() {
            out.push(Self::labelled(
                IconName::MemoryStick,
                p.good,
                "RAM:",
                s.ram.clone(),
                p,
            ));
        }
        if !s.disk.is_empty() {
            out.push(Self::labelled(
                IconName::HardDrive,
                p.warn,
                "Disk:",
                s.disk.clone(),
                p,
            ));
        }
        if !s.uptime.is_empty() {
            out.push(Self::labelled(
                IconName::Clock,
                p.accent,
                "Up:",
                s.uptime.clone(),
                p,
            ));
        }
        let dash = |v: &str| {
            if v.is_empty() {
                "\u{2014}".to_string()
            } else {
                v.to_string()
            }
        };
        out.push(
            Self::item(IconName::ArrowDown, p.good, p)
                .child(dash(&s.net_down))
                .into_any_element(),
        );
        out.push(
            Self::item(IconName::ArrowUp, p.accent, p)
                .child(dash(&s.net_up))
                .into_any_element(),
        );
        out
    }

    fn dot(color: Hsla) -> gpui_kit::Div {
        div().flex_none().size(px(6.)).rounded_full().bg(color)
    }
}

impl Render for StatusBar {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = *palette(cx);
        let mut left = h_flex().flex_1().min_w_0().gap_3().overflow_hidden();
        if self.disconnected {
            left = left.child(
                text_2xs(
                    h_flex()
                        .flex_none()
                        .gap_1()
                        .text_color(p.bad)
                        .whitespace_nowrap(),
                )
                .child(
                    Self::dot(p.bad).with_animation(
                        "status-disconnected",
                        Animation::new(Duration::from_secs(2))
                            .repeat()
                            .with_easing(pulsating_between(0.4, 1.0)),
                        |dot, delta| dot.opacity(delta),
                    ),
                )
                .child(div().font_weight(FontWeight::MEDIUM).child("Disconnected")),
            );
        } else if self.loading && self.status.load.is_empty() {
            left = left.child(
                text_2xs(
                    h_flex()
                        .flex_none()
                        .gap_1()
                        .text_color(p.ink_2)
                        .whitespace_nowrap(),
                )
                .child(Spinner::new().xsmall().color(p.ink_2))
                .child("Loading stats\u{2026}"),
            );
        } else {
            left = left.children(self.render_stats(&p));
        }
        if let Some(error) = &self.error {
            let tip: SharedString = error.clone().into();
            left = left.child(
                text_2xs(
                    h_flex()
                        .id("status-error")
                        .min_w_0()
                        .gap_1()
                        .text_color(p.bad),
                )
                .child(Self::dot(p.bad))
                .child(div().max_w(px(200.)).truncate().child(tip.clone()))
                .tooltip(move |window, cx| Tooltip::new(tip.clone()).build(window, cx)),
            );
        }
        let (chevron, chevron_label) = if self.expanded {
            (IconName::ChevronUp, "Hide system panel")
        } else {
            (IconName::ChevronDown, "Show system panel")
        };
        let bar = div()
            .id("host-status-bar")
            .flex_none()
            .border_t_1()
            .border_color(p.line)
            .bg(p.canvas)
            .cursor_pointer()
            .hover(move |s| s.bg(p.surface))
            .on_click(cx.listener(|this, _, _, cx| this.toggle(cx)))
            .child(
                h_flex()
                    .justify_between()
                    .px_2()
                    .py(px(2.))
                    .child(left)
                    .child(
                        // The whole bar toggles (Vue's button only stopped
                        // its click reaching the bar, which toggled too).
                        div()
                            .id("status-chevron")
                            .flex_none()
                            .ml_2()
                            .text_color(p.ink_2)
                            .hover(move |s| s.text_color(p.ink))
                            .child(Icon::new(chevron).size(px(12.)))
                            .tooltip(move |window, cx| {
                                Tooltip::new(chevron_label).build(window, cx)
                            }),
                    ),
            );
        gpui_kit::component::v_flex()
            .flex_none()
            .when_some(
                self.panel.clone().filter(|_| self.expanded),
                |col, panel| col.child(panel),
            )
            .child(bar)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // `useHostStatusPolling.test.js`.
    #[test]
    fn sums_rx_and_tx_across_interfaces_and_skips_loopback() {
        let netdev = "lo: 999 999\neth0: 100 20\nwlan0: 50 5\n";
        assert_eq!(compute_net_totals(netdev), (150, 25));
    }

    #[test]
    fn ignores_short_or_malformed_lines() {
        assert_eq!(compute_net_totals("eth0: 10\n\nbad line here x\n"), (0, 0));
        assert_eq!(compute_net_totals("eth0: abc def"), (0, 0));
        assert_eq!(compute_net_totals(""), (0, 0));
    }

    #[test]
    fn empty_status_has_every_field_blank() {
        let s = HostStatus::default();
        for v in [
            &s.load,
            &s.ram,
            &s.disk,
            &s.uptime,
            &s.os,
            &s.cores,
            &s.net_down,
            &s.net_up,
        ] {
            assert!(v.is_empty());
        }
    }

    #[test]
    fn parse_int_prefix_is_javascripts_parse_int() {
        assert_eq!(parse_int_prefix("123abc"), 123);
        assert_eq!(parse_int_prefix("abc"), 0);
        assert_eq!(parse_int_prefix("-5"), -5);
        assert_eq!(parse_int_prefix(""), 0);
    }

    fn stats(netdev: &str) -> serde_json::Value {
        json!({
            "load": "0.42", "ram": "1.2/7.6GB (16%)", "disk": "9.1G/58G (17%)",
            "uptime": "3d 4h 5m", "os": "Ubuntu 22.04.4 LTS", "kernel": "6.8.0",
            "arch": "x86_64", "cores": "8", "netdev": netdev,
        })
    }

    #[test]
    fn the_first_sample_shows_totals_then_rates() {
        let (first, net) = status_from_stats(
            &stats("lo: 5 5\neth0: 2048 1024"),
            NetSample::default(),
            10_000,
        );
        assert_eq!(first.net_down, "2.0 KB");
        assert_eq!(first.net_up, "1.0 KB");
        assert_eq!(first.os, "Ubuntu 22.04.4 LTS \u{b7} 6.8.0 \u{b7} x86_64");
        assert_eq!(first.load, "0.42");
        assert_eq!(first.cores, "8");
        let net = net.unwrap();
        assert_eq!(
            net,
            NetSample {
                rx: 2048,
                tx: 1024,
                time_ms: 10_000
            }
        );
        // Two seconds on: 4 KB more down, 512 B more up.
        let (next, _) = status_from_stats(&stats("eth0: 6144 1536"), net, 12_000);
        assert_eq!(next.net_down, "2.0 KB/s");
        assert_eq!(next.net_up, "256 B/s");
    }

    #[test]
    fn a_counter_reset_keeps_its_sign_and_no_netdev_keeps_the_counters() {
        let prev = NetSample {
            rx: 10_000,
            tx: 10_000,
            time_ms: 1_000,
        };
        let (s, _) = status_from_stats(&stats("eth0: 7952 10000"), prev, 2_000);
        assert_eq!(s.net_down, "-2.0 KB/s");
        let (s, net) = status_from_stats(&stats(""), prev, 2_000);
        assert_eq!((s.net_down.as_str(), s.net_up.as_str()), ("", ""));
        assert_eq!(net, None);
    }

    #[test]
    fn missing_parts_of_the_os_are_left_out() {
        let (s, _) = status_from_stats(
            &json!({"kernel": "6.1", "arch": "aarch64"}),
            NetSample::default(),
            1,
        );
        assert_eq!(s.os, "6.1 \u{b7} aarch64");
        assert_eq!(s.load, "");
    }

    #[test]
    fn the_error_loses_its_error_prefix() {
        assert_eq!(error_text("Error: boom"), "boom");
        assert_eq!(
            error_text("No active session for this host"),
            "No active session for this host"
        );
    }
}
