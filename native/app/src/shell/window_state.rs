//! The main window's size, position and maximized state, saved in the
//! `settings` table and restored at the next launch (plan, Phase 8:
//! "window bounds in the `settings` table").
//!
//! The key is the native app's own ([`WINDOW_STATE_KEY`]): the Tauri app's
//! window-state plugin keeps its bounds in a file of its own, and the two
//! windows have different minimum sizes and decorations, so sharing one
//! value would only move each app's window to where the other left it.
//!
//! The position is stored with the display it was on, as Zed does: GPUI's
//! window origin is relative to that display on macOS, and on every platform
//! a position on a monitor that is no longer attached would open the window
//! off screen. Without its display the window is centred at its saved size.
//! On Linux only the size and maximized state come back (see [`placement`]).

use std::sync::Arc;
use std::time::Duration;

use gpui_kit::{App, Bounds, Context, DisplayId, Pixels, Task, Window, WindowBounds, px, size};
use serde::{Deserialize, Serialize};

use crate::backend::HostStore;

/// The `settings` key. Native-only; see the module docs.
pub const WINDOW_STATE_KEY: &str = "native-window-state";

/// The size the window opens at when nothing is saved.
pub const DEFAULT_SIZE: (f32, f32) = (1200.0, 760.0);
/// The smallest the window may be, in either case.
pub const MIN_SIZE: (f32, f32) = (720.0, 420.0);

/// How long the window must stay still before its bounds are written: a drag
/// or a resize reports every frame.
const SAVE_DELAY: Duration = Duration::from_millis(400);

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WindowState {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    /// Maximized (or full screen) when it closed; `x`..`height` are then the
    /// bounds it restores to.
    #[serde(default)]
    pub maximized: bool,
    /// The display's persistent id (`PlatformDisplay::uuid`), when known.
    #[serde(default)]
    pub display: Option<String>,
}

impl WindowState {
    pub fn from_window(bounds: WindowBounds, display: Option<String>) -> Self {
        let (b, maximized) = match bounds {
            WindowBounds::Windowed(b) => (b, false),
            WindowBounds::Maximized(b) | WindowBounds::Fullscreen(b) => (b, true),
        };
        Self {
            x: f32::from(b.origin.x),
            y: f32::from(b.origin.y),
            width: f32::from(b.size.width),
            height: f32::from(b.size.height),
            maximized,
            display,
        }
    }

    /// The saved value, or `None` when it is missing, unreadable or not a
    /// usable size (NaN, zero, absurd).
    pub fn decode(raw: &str) -> Option<Self> {
        let s: Self = serde_json::from_str(raw).ok()?;
        let finite = [s.x, s.y, s.width, s.height].iter().all(|v| v.is_finite());
        let sane = (1.0..=100_000.0).contains(&s.width) && (1.0..=100_000.0).contains(&s.height);
        (finite && sane).then_some(s)
    }

    pub fn encode(&self) -> String {
        serde_json::to_string(self).unwrap_or_default()
    }

    /// The size to open at: at least the minimum, at most `fit` (the display).
    fn size(&self, fit: Option<(f32, f32)>) -> (f32, f32) {
        let (mut w, mut h) = (self.width.max(MIN_SIZE.0), self.height.max(MIN_SIZE.1));
        if let Some((fw, fh)) = fit {
            w = w.min(fw.max(MIN_SIZE.0));
            h = h.min(fh.max(MIN_SIZE.1));
        }
        (w, h)
    }
}

/// A display the window could open on.
#[derive(Debug, Clone)]
pub struct DisplayInfo {
    pub id: DisplayId,
    pub uuid: Option<String>,
    pub size: (f32, f32),
}

/// Where the window opens: the saved bounds on the saved display, or the
/// saved size centred on the main display when that display is gone.
/// `None` when nothing usable was saved; the caller centres the default.
///
/// `restore_position` is false on Linux ([`RESTORE_POSITION`]): Wayland lets
/// no client place its window, and under X11 the origin GPUI reports is the
/// client area's while the one it opens at is the frame's, so a restored
/// window crept down and right by the decoration size on every launch
/// (measured under WSLg: 8 and 27 pixels). The size and maximized state are
/// restored everywhere.
pub fn placement(
    saved: Option<&WindowState>,
    displays: &[DisplayInfo],
    restore_position: bool,
) -> Option<Placement> {
    let saved = saved?;
    let display = saved
        .display
        .as_ref()
        .and_then(|want| displays.iter().find(|d| d.uuid.as_ref() == Some(want)));
    let (w, h) = saved.size(display.map(|d| d.size));
    match display {
        Some(d) => Some(Placement {
            display: Some(d.id),
            origin: restore_position.then_some((saved.x, saved.y)),
            size: (w, h),
            maximized: saved.maximized,
        }),
        None => Some(Placement {
            display: None,
            origin: None,
            size: (w, h),
            maximized: saved.maximized,
        }),
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Placement {
    pub display: Option<DisplayId>,
    /// `None`: centre on the display.
    pub origin: Option<(f32, f32)>,
    pub size: (f32, f32),
    pub maximized: bool,
}

/// Whether the saved position is used on this platform; see [`placement`].
pub const RESTORE_POSITION: bool = !cfg!(target_os = "linux");

/// The displays attached now.
pub fn displays(cx: &App) -> Vec<DisplayInfo> {
    cx.displays()
        .into_iter()
        .map(|d| {
            let b = d.bounds();
            DisplayInfo {
                id: d.id(),
                uuid: d.uuid().ok().map(|u| u.to_string()),
                size: (f32::from(b.size.width), f32::from(b.size.height)),
            }
        })
        .collect()
}

/// The `WindowOptions` fields for the saved state: bounds and display.
pub fn window_bounds(saved: Option<&WindowState>, cx: &App) -> (WindowBounds, Option<DisplayId>) {
    let default = || {
        WindowBounds::Windowed(Bounds::centered(
            None,
            size(px(DEFAULT_SIZE.0), px(DEFAULT_SIZE.1)),
            cx,
        ))
    };
    let Some(p) = placement(saved, &displays(cx), RESTORE_POSITION) else {
        return (default(), None);
    };
    let sz = size(px(p.size.0), px(p.size.1));
    let bounds: Bounds<Pixels> = match p.origin {
        Some((x, y)) => Bounds::new(gpui_kit::point(px(x), px(y)), sz),
        None => Bounds::centered(p.display, sz, cx),
    };
    let wb = if p.maximized {
        WindowBounds::Maximized(bounds)
    } else {
        WindowBounds::Windowed(bounds)
    };
    (wb, p.display)
}

/// Writes the window's state a moment after it stops changing, and once more
/// when the window goes away, straight to the store (no other view shows it).
pub struct WindowStateSaver {
    store: Arc<dyn HostStore>,
    last_saved: Option<WindowState>,
    pending: Option<WindowState>,
    _timer: Option<Task<()>>,
}

impl WindowStateSaver {
    pub fn new(store: Arc<dyn HostStore>) -> Self {
        let last_saved = store
            .get_setting(WINDOW_STATE_KEY)
            .ok()
            .flatten()
            .and_then(|raw| WindowState::decode(&raw));
        Self {
            store,
            last_saved,
            pending: None,
            _timer: None,
        }
    }

    /// The window moved, resized, or (un)maximized. `cx` is the view that
    /// owns the saver; `saver` picks it out of that view for the timer.
    pub fn changed<V: 'static>(
        &mut self,
        window: &Window,
        cx: &mut Context<V>,
        saver: fn(&mut V) -> &mut WindowStateSaver,
    ) {
        let display = window
            .display(cx)
            .and_then(|d| d.uuid().ok())
            .map(|u| u.to_string());
        let mut state = WindowState::from_window(window.window_bounds(), display);
        // Some platforms report a maximized window as Windowed at the size
        // of the display; ask directly.
        if window.is_maximized() {
            state.maximized = true;
        }
        if self.last_saved.as_ref() == Some(&state) {
            self.pending = None;
            return;
        }
        self.pending = Some(state);
        self._timer = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(SAVE_DELAY).await;
            let _ = this.update(cx, |view, _| saver(view).flush());
        }));
    }

    /// Write what is pending now.
    pub fn flush(&mut self) {
        let Some(state) = self.pending.take() else {
            return;
        };
        match self.store.set_setting(WINDOW_STATE_KEY, &state.encode()) {
            Ok(()) => self.last_saved = Some(state),
            Err(e) => tracing::warn!("Could not save the window state: {e}"),
        }
    }
}

impl Drop for WindowStateSaver {
    fn drop(&mut self) {
        self.flush();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state(display: Option<&str>) -> WindowState {
        WindowState {
            x: 100.0,
            y: 50.0,
            width: 1000.0,
            height: 700.0,
            maximized: false,
            display: display.map(str::to_string),
        }
    }

    fn display(id: u64, uuid: &str, w: f32, h: f32) -> DisplayInfo {
        DisplayInfo {
            id: DisplayId::new(id),
            uuid: Some(uuid.into()),
            size: (w, h),
        }
    }

    #[test]
    fn round_trips_through_the_setting() {
        let s = state(Some("abc"));
        assert_eq!(WindowState::decode(&s.encode()), Some(s));
    }

    #[test]
    fn rejects_values_that_are_not_a_window() {
        assert_eq!(WindowState::decode(""), None);
        assert_eq!(WindowState::decode("{}"), None);
        assert_eq!(
            WindowState::decode(r#"{"x":0,"y":0,"width":0,"height":500}"#),
            None
        );
        assert_eq!(
            WindowState::decode(r#"{"x":0,"y":0,"width":1e9,"height":500}"#),
            None
        );
        // Older or hand-written values without the optional fields still load.
        assert!(WindowState::decode(r#"{"x":-5,"y":3,"width":800,"height":500}"#).is_some());
    }

    #[test]
    fn maximized_and_full_screen_keep_their_restore_bounds() {
        let b = Bounds::new(gpui_kit::point(px(10.), px(20.)), size(px(900.), px(600.)));
        let s = WindowState::from_window(WindowBounds::Maximized(b), None);
        assert!(s.maximized);
        assert_eq!((s.x, s.y, s.width, s.height), (10.0, 20.0, 900.0, 600.0));
        assert!(WindowState::from_window(WindowBounds::Fullscreen(b), None).maximized);
        assert!(!WindowState::from_window(WindowBounds::Windowed(b), None).maximized);
    }

    #[test]
    fn nothing_saved_opens_at_the_default() {
        assert_eq!(
            placement(None, &[display(1, "a", 1920., 1080.)], true),
            None
        );
    }

    #[test]
    fn the_saved_display_gets_the_saved_position() {
        let d = [display(1, "a", 1920., 1080.), display(2, "b", 2560., 1440.)];
        let p = placement(Some(&state(Some("b"))), &d, true).unwrap();
        assert_eq!(p.display, Some(DisplayId::new(2)));
        assert_eq!(p.origin, Some((100.0, 50.0)));
        assert_eq!(p.size, (1000.0, 700.0));
    }

    #[test]
    fn without_restore_position_only_the_size_and_display_are_kept() {
        let d = [display(1, "a", 1920., 1080.)];
        let mut s = state(Some("a"));
        s.maximized = true;
        let p = placement(Some(&s), &d, false).unwrap();
        assert_eq!(p.display, Some(DisplayId::new(1)));
        assert_eq!(p.origin, None);
        assert_eq!(p.size, (1000.0, 700.0));
        assert!(p.maximized);
    }

    #[test]
    fn a_detached_display_centres_the_window_instead() {
        let d = [display(1, "a", 1920., 1080.)];
        let p = placement(Some(&state(Some("gone"))), &d, true).unwrap();
        assert_eq!(p.display, None);
        assert_eq!(p.origin, None);
        assert_eq!(p.size, (1000.0, 700.0));
        // And so does a value saved without a display.
        assert_eq!(
            placement(Some(&state(None)), &d, true).unwrap().origin,
            None
        );
    }

    #[test]
    fn the_size_is_kept_within_the_display_and_above_the_minimum() {
        let mut s = state(Some("a"));
        s.width = 5000.0;
        s.height = 100.0;
        let p = placement(Some(&s), &[display(1, "a", 1920., 1080.)], true).unwrap();
        assert_eq!(p.size, (1920.0, MIN_SIZE.1));
    }
}
