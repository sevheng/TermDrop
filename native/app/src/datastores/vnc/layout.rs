//! Geometry for the remote screen: how big to draw it, and where a click
//! landed. Port of `src/utils/vncLayout.js`, plus the native app's panning
//! for the 1:1 mode (see [`actual_offset`]).

/// Fit the whole framebuffer into the viewport, letterboxed.
pub const FIT: &str = "fit";
/// One remote pixel per device pixel.
pub const ACTUAL: &str = "actual";

/// The two display modes; stored as the `vnc.fitMode` setting, shared with
/// the Tauri app.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FitMode {
    Fit,
    Actual,
}

impl FitMode {
    pub fn from_setting(value: Option<&str>) -> Option<Self> {
        match value {
            Some(FIT) => Some(Self::Fit),
            Some(ACTUAL) => Some(Self::Actual),
            _ => None,
        }
    }

    pub fn setting(self) -> &'static str {
        match self {
            Self::Fit => FIT,
            Self::Actual => ACTUAL,
        }
    }

    pub fn toggled(self) -> Self {
        match self {
            Self::Fit => Self::Actual,
            Self::Actual => Self::Fit,
        }
    }
}

/// The size to draw the framebuffer at, in logical pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Layout {
    pub width: f32,
    pub height: f32,
    pub scale: f32,
    pub letterboxed: bool,
}

/// `vncLayout`. `dpr` divides rather than multiplies: the framebuffer's
/// resolution is fixed by the server, so 1:1 maps one remote pixel onto one
/// *device* pixel, which is as sharp as the source allows. Fit never
/// enlarges: a 640x480 desktop blown up to fill a 4K window looks like a
/// fault.
pub fn layout(fb_w: f32, fb_h: f32, view_w: f32, view_h: f32, mode: FitMode, dpr: f32) -> Layout {
    let dpr = if dpr > 0.0 && dpr.is_finite() {
        dpr
    } else {
        1.0
    };
    if fb_w <= 0.0 || fb_h <= 0.0 {
        return Layout {
            width: 0.0,
            height: 0.0,
            scale: 1.0,
            letterboxed: false,
        };
    }
    if mode == FitMode::Actual {
        return Layout {
            width: fb_w / dpr,
            height: fb_h / dpr,
            scale: 1.0 / dpr,
            letterboxed: false,
        };
    }
    let scale = (view_w / fb_w).min(view_h / fb_h).clamp(0.0, 1.0);
    Layout {
        width: fb_w * scale,
        height: fb_h * scale,
        scale,
        letterboxed: fb_w * scale < view_w || fb_h * scale < view_h,
    }
}

/// Where the image starts inside the viewport, on one axis.
///
/// Smaller than the viewport: centred, as the Vue panel's flex box did.
/// Larger (1:1 on a desktop bigger than the window): the Vue panel scrolled
/// with scrollbars, the wheel going to the remote. Here the image follows
/// the pointer instead: with the pointer at a fraction `f` across the
/// viewport, the image is offset by `f` of the overflow, so every remote
/// pixel can be reached by moving the mouse and no scrollbar is needed.
pub fn actual_offset(content: f32, view: f32, pointer_fraction: f32) -> f32 {
    if content <= view {
        return (view - content) / 2.0;
    }
    -(content - view) * pointer_fraction.clamp(0.0, 1.0)
}

/// `pointerToFramebuffer`: a window position to a framebuffer pixel, through
/// the rectangle the image is drawn in right now. Clamped to the last real
/// pixel; a zero-sized image maps everything to the origin.
pub fn pointer_to_framebuffer(
    origin: (f32, f32),
    size: (f32, f32),
    fb: (u16, u16),
    pointer: (f32, f32),
) -> (u16, u16) {
    let (w, h) = size;
    if w <= 0.0 || h <= 0.0 {
        return (0, 0);
    }
    let x = ((pointer.0 - origin.0) * fb.0 as f32 / w).round();
    let y = ((pointer.1 - origin.1) * fb.1 as f32 / h).round();
    let clamp = |v: f32, len: u16| v.clamp(0.0, len.saturating_sub(1) as f32) as u16;
    (clamp(x, fb.0), clamp(y, fb.1))
}

/// Pixels of scroll that make up one wheel click on the remote.
const TICK_PX: f32 = 50.0;
/// Line deltas: the assumed line height.
const LINE_PX: f32 = 16.0;

/// A wheel delta, in the units the platform gave it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum WheelDelta {
    Pixels(f32),
    Lines(f32),
}

/// `wheelTicks`: accumulate wheel deltas into whole clicks, carrying the
/// remainder so a slow trackpad gesture still adds up to one. Positive is
/// down/right, as the DOM's `deltaY`. Returns `(ticks, remainder)`.
pub fn wheel_ticks(accum: f32, delta: WheelDelta) -> (i32, f32) {
    let px = match delta {
        WheelDelta::Pixels(p) => p,
        WheelDelta::Lines(l) => l * LINE_PX,
    };
    let total = accum + px;
    let ticks = (total / TICK_PX).trunc();
    (ticks as i32, total - ticks * TICK_PX)
}

/// Mouse buttons held, as RFB numbers them: left, middle, right as bits
/// 0, 1, 2. (The DOM order is left/right/middle, which every web client
/// transposes once; GPUI names the buttons, so there is nothing to swap.)
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Buttons {
    pub left: bool,
    pub middle: bool,
    pub right: bool,
}

impl Buttons {
    pub fn mask(self) -> u8 {
        u8::from(self.left) | u8::from(self.middle) << 1 | u8::from(self.right) << 2
    }

    pub fn any(self) -> bool {
        self.mask() != 0
    }
}

/// RFB wheel buttons: up, down, left, right.
pub const WHEEL_UP: u8 = 1 << 3;
pub const WHEEL_DOWN: u8 = 1 << 4;
pub const WHEEL_LEFT: u8 = 1 << 5;
pub const WHEEL_RIGHT: u8 = 1 << 6;

/// The pointer events for `ticks` wheel clicks: RFB has no scroll event, so
/// a click is a press and release of a wheel button, with whatever buttons
/// are held kept held.
pub fn wheel_events(ticks: i32, positive: u8, negative: u8, held: u8) -> Vec<u8> {
    let button = if ticks > 0 { positive } else { negative };
    (0..ticks.unsigned_abs())
        .flat_map(|_| [held | button, held])
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-4
    }

    #[test]
    fn contains_the_framebuffer_in_the_viewport() {
        let l = layout(1920., 1080., 960., 1000., FitMode::Fit, 1.);
        assert!(close(l.scale, 0.5));
        assert!(close(l.width, 960.) && close(l.height, 540.));
        assert!(l.letterboxed);
    }

    #[test]
    fn never_enlarges_past_one_to_one() {
        let l = layout(640., 480., 3840., 2160., FitMode::Fit, 1.);
        assert_eq!(l.scale, 1.);
        assert_eq!(l.width, 640.);
    }

    #[test]
    fn divides_by_the_device_pixel_ratio_at_actual_size() {
        let l = layout(1920., 1080., 800., 600., FitMode::Actual, 2.);
        assert_eq!((l.width, l.height), (960., 540.));
        let l = layout(1024., 768., 500., 500., FitMode::Actual, 1.);
        assert_eq!((l.width, l.height), (1024., 768.));
    }

    #[test]
    fn survives_a_zero_framebuffer_or_a_nonsense_ratio() {
        assert_eq!(layout(0., 0., 100., 100., FitMode::Fit, 1.).width, 0.);
        let l = layout(100., 100., 50., 50., FitMode::Actual, 0.);
        assert!(l.width.is_finite());
    }

    #[test]
    fn the_fit_mode_round_trips_through_the_setting() {
        for mode in [FitMode::Fit, FitMode::Actual] {
            assert_eq!(FitMode::from_setting(Some(mode.setting())), Some(mode));
            assert_eq!(mode.toggled().toggled(), mode);
        }
        assert_eq!(FitMode::from_setting(Some("zoom")), None);
        assert_eq!(FitMode::from_setting(None), None);
    }

    #[test]
    fn scales_a_click_through_the_drawn_rectangle() {
        let (o, s) = ((10., 20.), (960., 540.));
        assert_eq!(
            pointer_to_framebuffer(o, s, (1920, 1080), (10., 20.)),
            (0, 0)
        );
        assert_eq!(
            pointer_to_framebuffer(o, s, (1920, 1080), (490., 290.)),
            (960, 540)
        );
    }

    #[test]
    fn clamps_to_the_last_real_pixel() {
        let (o, s) = ((10., 20.), (960., 540.));
        assert_eq!(
            pointer_to_framebuffer(o, s, (1920, 1080), (970., 560.)),
            (1919, 1079)
        );
        assert_eq!(
            pointer_to_framebuffer(o, s, (1920, 1080), (-100., -100.)),
            (0, 0)
        );
    }

    #[test]
    fn does_not_divide_by_a_zero_sized_image() {
        assert_eq!(
            pointer_to_framebuffer((0., 0.), (0., 0.), (100, 100), (5., 5.)),
            (0, 0)
        );
    }

    #[test]
    fn a_small_image_is_centred_and_a_large_one_follows_the_pointer() {
        assert_eq!(actual_offset(100., 300., 0.9), 100.);
        assert_eq!(actual_offset(1000., 400., 0.), 0.);
        assert_eq!(actual_offset(1000., 400., 1.), -600.);
        assert_eq!(actual_offset(1000., 400., 0.5), -300.);
        assert_eq!(actual_offset(1000., 400., 7.), -600.);
    }

    #[test]
    fn wheel_carries_a_remainder_so_a_slow_gesture_still_scrolls() {
        let (t, r) = wheel_ticks(0., WheelDelta::Pixels(20.));
        assert_eq!(t, 0);
        let (t, r) = wheel_ticks(r, WheelDelta::Pixels(20.));
        assert_eq!(t, 0);
        let (t, r) = wheel_ticks(r, WheelDelta::Pixels(20.));
        assert_eq!(t, 1);
        assert!(close(r, 10.));
    }

    #[test]
    fn wheel_emits_several_clicks_both_ways() {
        assert_eq!(wheel_ticks(0., WheelDelta::Pixels(160.)).0, 3);
        assert_eq!(wheel_ticks(0., WheelDelta::Pixels(-120.)).0, -2);
    }

    #[test]
    fn wheel_converts_lines_to_pixels() {
        assert_eq!(wheel_ticks(0., WheelDelta::Lines(3.)).0, 0);
        assert_eq!(wheel_ticks(0., WheelDelta::Lines(12.)).0, 3);
    }

    #[test]
    fn buttons_use_the_rfb_order() {
        let b = |left, middle, right| {
            Buttons {
                left,
                middle,
                right,
            }
            .mask()
        };
        assert_eq!(b(true, false, false), 0b001);
        assert_eq!(b(false, true, false), 0b010);
        assert_eq!(b(false, false, true), 0b100);
        assert_eq!(b(true, false, true), 0b101);
        assert_eq!(b(false, false, false), 0);
    }

    #[test]
    fn a_wheel_click_is_a_press_and_release_keeping_held_buttons() {
        assert_eq!(
            wheel_events(2, WHEEL_DOWN, WHEEL_UP, 1),
            vec![1 | WHEEL_DOWN, 1, 1 | WHEEL_DOWN, 1]
        );
        assert_eq!(wheel_events(-1, WHEEL_DOWN, WHEEL_UP, 0), vec![WHEEL_UP, 0]);
        assert!(wheel_events(0, WHEEL_DOWN, WHEEL_UP, 0).is_empty());
    }
}
