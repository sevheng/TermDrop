//! TermDrop's design tokens, ported from `src/themes/tokens.css`, and their
//! projection onto GPUI Kit's theme.
//!
//! Tokens are RGB channels, as in the CSS, and both themes answer to the same
//! names. They reach the screen two ways:
//!
//! - **GPUI Kit's `Theme`.** [`install`] registers a light and a dark
//!   `ThemeConfig` built from the tokens, so every Kit component (buttons,
//!   inputs, menus, dialogs, notifications) follows them with no per-site
//!   styling. This is the equivalent of Tailwind's `@theme inline` block.
//! - **[`Palette`].** The roles Kit has no slot for (`ink-2`, `ink-3`,
//!   `good`, `warn`, `redis`, ...) live in a global our own views read. As in
//!   the Vue app, a view names a token, never a colour.
//!
//! Two rules carry over from the CSS. A bright tone cannot carry a white
//! label (`bg-accent text-white` measured 2.79:1), so filled controls use the
//! `-solid` variants. And every text tone clears WCAG AA on every surface in
//! both themes, which is why these values are copied rather than tweaked.

use std::rc::Rc;

use gpui_kit::component::{Theme, ThemeConfig, ThemeMode, ThemeSet};
use gpui_kit::{App, Global, Hsla, Rgba, SharedString};

use crate::app::prefs::ThemeChoice;

type Rgb = [u8; 3];

/// One theme's tokens, named as in `tokens.css` with `-` as `_`.
#[derive(Debug, Clone, Copy)]
pub struct Tokens {
    pub canvas: Rgb,
    pub surface: Rgb,
    pub overlay: Rgb,
    pub raised: Rgb,
    pub active: Rgb,
    pub selected: Rgb,
    pub input: Rgb,
    pub input_hover: Rgb,
    pub line: Rgb,
    pub ink: Rgb,
    pub ink_2: Rgb,
    pub ink_3: Rgb,
    pub placeholder: Rgb,
    pub accent: Rgb,
    pub accent_hover: Rgb,
    pub accent_solid: Rgb,
    pub accent_solid_hover: Rgb,
    pub good: Rgb,
    pub good_solid: Rgb,
    pub good_solid_hover: Rgb,
    pub warn: Rgb,
    pub warn_solid: Rgb,
    pub warn_solid_hover: Rgb,
    pub bad: Rgb,
    pub bad_solid: Rgb,
    pub bad_solid_hover: Rgb,
    pub warn_bg: Rgb,
    pub warn_line: Rgb,
    pub bad_bg: Rgb,
    pub bad_line: Rgb,
    pub redis: Rgb,
    pub mongo: Rgb,
    pub vnc: Rgb,
    /// Data syntax tones (`--td-syn-*`): JSON keys, types, values.
    pub syn_blue: Rgb,
    pub syn_cyan: Rgb,
    pub syn_yellow: Rgb,
    pub syn_teal: Rgb,
    pub syn_purple: Rgb,
    /// `--td-tag-1` .. `--td-tag-8`: the eight group-rail hues, each clearing
    /// 3:1 against the sidebar surface. Picked by
    /// [`crate::app::group_accent::group_accent_index`].
    pub tag: [Rgb; 8],
}

pub const LIGHT: Tokens = Tokens {
    canvas: [250, 251, 253],
    surface: [235, 239, 245],
    overlay: [255, 255, 255],
    raised: [223, 229, 238],
    active: [225, 231, 240],
    selected: [217, 231, 248],
    input: [255, 255, 255],
    input_hover: [225, 231, 240],
    line: [210, 218, 228],
    ink: [20, 26, 34],
    ink_2: [85, 96, 112],
    ink_3: [92, 102, 117],
    placeholder: [156, 163, 175],
    accent: [20, 97, 196],
    accent_hover: [15, 78, 158],
    accent_solid: [20, 97, 196],
    accent_solid_hover: [15, 78, 158],
    good: [15, 95, 51],
    good_solid: [20, 122, 59],
    good_solid_hover: [15, 95, 46],
    warn: [122, 79, 0],
    warn_solid: [122, 79, 0],
    warn_solid_hover: [94, 61, 0],
    bad: [179, 32, 23],
    bad_solid: [179, 32, 23],
    bad_solid_hover: [143, 25, 18],
    warn_bg: [253, 244, 224],
    warn_line: [232, 209, 154],
    bad_bg: [252, 234, 232],
    bad_line: [240, 188, 182],
    redis: [179, 32, 23],
    mongo: [21, 99, 58],
    vnc: [109, 40, 217],
    syn_blue: [26, 95, 168],
    syn_cyan: [24, 108, 134],
    syn_yellow: [122, 84, 16],
    syn_teal: [19, 107, 92],
    syn_purple: [122, 62, 130],
    tag: [
        [44, 107, 181],
        [37, 121, 76],
        [107, 79, 181],
        [168, 95, 33],
        [138, 109, 0],
        [176, 58, 58],
        [27, 124, 119],
        [168, 74, 128],
    ],
};

/// `html.dark` in the CSS. The accent here is the brand `#3B9EFF`.
pub const DARK: Tokens = Tokens {
    canvas: [15, 17, 22],
    surface: [22, 25, 32],
    overlay: [37, 42, 52],
    raised: [30, 34, 43],
    active: [37, 42, 52],
    selected: [27, 58, 92],
    input: [30, 34, 43],
    input_hover: [42, 48, 59],
    line: [42, 48, 59],
    ink: [231, 234, 240],
    ink_2: [154, 164, 178],
    ink_3: [135, 145, 160],
    placeholder: [156, 163, 175],
    accent: [59, 158, 255],
    accent_hover: [92, 175, 255],
    accent_solid: [27, 98, 189],
    accent_solid_hover: [42, 120, 216],
    good: [74, 222, 128],
    good_solid: [21, 128, 61],
    good_solid_hover: [26, 147, 73],
    warn: [251, 191, 36],
    warn_solid: [138, 90, 0],
    warn_solid_hover: [161, 108, 0],
    bad: [248, 113, 113],
    bad_solid: [180, 35, 31],
    bad_solid_hover: [201, 48, 40],
    warn_bg: [46, 38, 23],
    warn_line: [77, 64, 35],
    bad_bg: [46, 26, 28],
    bad_line: [95, 42, 45],
    redis: [240, 83, 63],
    mongo: [77, 208, 122],
    vnc: [167, 139, 250],
    syn_blue: [127, 182, 236],
    syn_cyan: [165, 214, 245],
    syn_yellow: [227, 192, 139],
    syn_teal: [111, 211, 190],
    syn_purple: [211, 162, 216],
    tag: [
        [79, 143, 224],
        [79, 176, 122],
        [155, 127, 224],
        [217, 136, 74],
        [201, 162, 39],
        [217, 106, 106],
        [63, 175, 168],
        [217, 122, 176],
    ],
};

pub fn tokens(choice: ThemeChoice) -> &'static Tokens {
    match choice {
        ThemeChoice::Dark => &DARK,
        ThemeChoice::Light => &LIGHT,
    }
}

fn hex([r, g, b]: Rgb) -> String {
    format!("#{r:02x}{g:02x}{b:02x}")
}

fn hex_a(rgb: Rgb, alpha: u8) -> String {
    format!("{}{alpha:02x}", hex(rgb))
}

fn hsla([r, g, b]: Rgb) -> Hsla {
    Rgba {
        r: f32::from(r) / 255.,
        g: f32::from(g) / 255.,
        b: f32::from(b) / 255.,
        a: 1.,
    }
    .into()
}

pub use super::fonts::{MONO_FONT, UI_FONT};

/// A GPUI Kit theme config for one mode, every Kit colour slot mapped to the
/// token that plays that role in the Vue app.
pub fn theme_config_json(choice: ThemeChoice, font_family: Option<&str>) -> serde_json::Value {
    let t = tokens(choice);
    let white = "#ffffff";
    let colors = serde_json::json!({
        "background": hex(t.canvas),
        "foreground": hex(t.ink),
        "border": hex(t.line),
        "input.border": hex(t.line),
        "ring": hex(t.accent),
        "caret": hex(t.ink),
        "selection.background": hex_a(t.accent, 0x55),
        "overlay": "#00000099",
        "window.border": hex(t.line),
        "drag.border": hex(t.accent),
        "drop_target.background": hex_a(t.accent, 0x33),

        // Menu-item hover and other quiet emphasis.
        "accent.background": hex(t.raised),
        "accent.foreground": hex(t.ink),
        "muted.background": hex(t.raised),
        "muted.foreground": hex(t.ink_2),

        // Fills that carry a white label use the -solid tones.
        "primary.background": hex(t.accent_solid),
        "primary.hover.background": hex(t.accent_solid_hover),
        "primary.active.background": hex(t.accent_solid_hover),
        "primary.foreground": white,
        "secondary.background": hex(t.raised),
        "secondary.hover.background": hex(t.input_hover),
        "secondary.active.background": hex(t.active),
        "secondary.foreground": hex(t.ink),
        "danger.background": hex(t.bad_solid),
        "danger.hover.background": hex(t.bad_solid_hover),
        "danger.active.background": hex(t.bad_solid_hover),
        "danger.foreground": white,
        "success.background": hex(t.good_solid),
        "success.hover.background": hex(t.good_solid_hover),
        "success.active.background": hex(t.good_solid_hover),
        "success.foreground": white,
        "warning.background": hex(t.warn_solid),
        "warning.hover.background": hex(t.warn_solid_hover),
        "warning.active.background": hex(t.warn_solid_hover),
        "warning.foreground": white,
        "info.background": hex(t.accent_solid),
        "info.hover.background": hex(t.accent_solid_hover),
        "info.active.background": hex(t.accent_solid_hover),
        "info.foreground": white,

        // Kit's buttons do not fall back to the fills above: unset, a danger
        // button is a translucent tint with the tone as its label, which in
        // dark measured as red on dark red. Every filled button is a -solid
        // fill with a white label instead.
        "button.primary.background": hex(t.accent_solid),
        "button.primary.hover.background": hex(t.accent_solid_hover),
        "button.primary.active.background": hex(t.accent_solid_hover),
        "button.primary.foreground": white,
        "button.danger.background": hex(t.bad_solid),
        "button.danger.hover.background": hex(t.bad_solid_hover),
        "button.danger.active.background": hex(t.bad_solid_hover),
        "button.danger.foreground": white,
        "button.success.background": hex(t.good_solid),
        "button.success.hover.background": hex(t.good_solid_hover),
        "button.success.active.background": hex(t.good_solid_hover),
        "button.success.foreground": white,
        "button.warning.background": hex(t.warn_solid),
        "button.warning.hover.background": hex(t.warn_solid_hover),
        "button.warning.active.background": hex(t.warn_solid_hover),
        "button.warning.foreground": white,
        "button.info.background": hex(t.accent_solid),
        "button.info.hover.background": hex(t.accent_solid_hover),
        "button.info.active.background": hex(t.accent_solid_hover),
        "button.info.foreground": white,
        "button.secondary.background": hex(t.raised),
        "button.secondary.hover.background": hex(t.input_hover),
        "button.secondary.active.background": hex(t.active),
        "button.secondary.foreground": hex(t.ink),

        "popover.background": hex(t.overlay),
        "popover.foreground": hex(t.ink),
        "list.background": hex(t.surface),
        "list.hover.background": hex(t.raised),
        "list.active.background": hex(t.selected),
        "list.active.border": hex(t.accent),
        "list.even.background": hex(t.surface),
        "list.head.background": hex(t.surface),
        "table.background": hex(t.canvas),
        "table.head.background": hex(t.surface),
        "table.head.foreground": hex(t.ink_2),
        "table.hover.background": hex(t.raised),
        "table.active.background": hex(t.selected),
        "table.active.border": hex(t.accent),
        "table.even.background": hex(t.canvas),
        "table.row.border": hex(t.line),

        "sidebar.background": hex(t.surface),
        "sidebar.foreground": hex(t.ink),
        "sidebar.border": hex(t.line),
        "sidebar.accent.background": hex(t.raised),
        "sidebar.accent.foreground": hex(t.ink),
        "sidebar.primary.background": hex(t.accent_solid),
        "sidebar.primary.foreground": white,

        "tab_bar.background": hex(t.surface),
        "tab_bar.segmented.background": hex(t.raised),
        "tab.background": hex(t.surface),
        "tab.foreground": hex(t.ink_2),
        "tab.active.background": hex(t.active),
        "tab.active.foreground": hex(t.ink),
        "title_bar.background": hex(t.surface),
        "title_bar.border": hex(t.line),
        "status_bar.background": hex(t.surface),
        "status_bar.border": hex(t.line),

        "link": hex(t.accent),
        "link.hover": hex(t.accent_hover),
        "link.active": hex(t.accent_hover),
        "switch.background": hex(t.raised),
        "switch.thumb.background": white,
        "slider.background": hex(t.accent_solid),
        "slider.thumb.background": white,
        "progress.bar.background": hex(t.accent_solid),
        "skeleton.background": hex(t.raised),
        "scrollbar.background": "#00000000",
        "scrollbar.thumb.background": hex_a(t.ink_3, 0x88),
        "scrollbar.thumb.hover.background": hex(t.ink_3),
        "accordion.background": hex(t.surface),
        "group_box.background": hex(t.surface),
        "group_box.foreground": hex(t.ink),
        "description_list.label.background": hex(t.surface),
        "description_list.label.foreground": hex(t.ink_2),
        "chart.1": hex(t.accent),
        "chart.2": hex(t.good),
        "chart.3": hex(t.warn),
        "chart.4": hex(t.bad),
        "chart.5": hex(t.vnc),
        "chart.bullish": hex(t.good),
        "chart.bearish": hex(t.bad),
        "chart.grid": hex(t.line),
        "base.blue": hex(t.accent),
        "base.green": hex(t.good),
        "base.red": hex(t.bad),
        "base.yellow": hex(t.warn),
        "base.magenta": hex(t.vnc),
        "base.cyan": hex(t.accent_hover),
    });
    let mut config = serde_json::json!({
        "name": match choice {
            ThemeChoice::Dark => "TermDrop Dark",
            ThemeChoice::Light => "TermDrop Light",
        },
        "mode": choice.as_str(),
        "is_default": true,
        // The Vue app's 13px `text-sm` is the body size of most controls.
        "font.size": 14.0,
        "radius": 4,
        "radius.lg": 8,
        "colors": colors,
    });
    if let Some(family) = font_family {
        config["font.family"] = family.into();
    }
    // Code in the UI (paths, fingerprints, documents) in the terminal's face.
    config["mono_font.family"] = MONO_FONT.into();
    config
}

fn theme_config(choice: ThemeChoice, font_family: Option<&str>) -> Rc<ThemeConfig> {
    let set = serde_json::json!({
        "name": "TermDrop",
        "themes": [theme_config_json(choice, font_family)],
    });
    let mut set: ThemeSet =
        serde_json::from_value(set).expect("the TermDrop theme is valid by construction");
    Rc::new(set.themes.remove(0))
}

/// Roles GPUI Kit has no slot for, for our own views.
#[derive(Debug, Clone, Copy)]
pub struct Palette {
    pub canvas: Hsla,
    pub surface: Hsla,
    pub overlay: Hsla,
    pub raised: Hsla,
    pub active: Hsla,
    pub selected: Hsla,
    pub input: Hsla,
    pub line: Hsla,
    pub ink: Hsla,
    pub ink_2: Hsla,
    pub ink_3: Hsla,
    pub placeholder: Hsla,
    pub accent: Hsla,
    pub accent_solid: Hsla,
    pub good: Hsla,
    pub warn: Hsla,
    pub bad: Hsla,
    pub bad_solid: Hsla,
    pub warn_bg: Hsla,
    pub warn_line: Hsla,
    pub bad_bg: Hsla,
    pub bad_line: Hsla,
    pub redis: Hsla,
    pub mongo: Hsla,
    pub vnc: Hsla,
    pub syn_blue: Hsla,
    pub syn_cyan: Hsla,
    pub syn_yellow: Hsla,
    pub syn_teal: Hsla,
    pub syn_purple: Hsla,
    /// The group-rail hues, `tag[group_accent_index(name)]`.
    pub tag: [Hsla; 8],
}

impl Global for Palette {}

impl Palette {
    pub fn from_tokens(t: &Tokens) -> Self {
        Self {
            canvas: hsla(t.canvas),
            surface: hsla(t.surface),
            overlay: hsla(t.overlay),
            raised: hsla(t.raised),
            active: hsla(t.active),
            selected: hsla(t.selected),
            input: hsla(t.input),
            line: hsla(t.line),
            ink: hsla(t.ink),
            ink_2: hsla(t.ink_2),
            ink_3: hsla(t.ink_3),
            placeholder: hsla(t.placeholder),
            accent: hsla(t.accent),
            accent_solid: hsla(t.accent_solid),
            good: hsla(t.good),
            warn: hsla(t.warn),
            bad: hsla(t.bad),
            bad_solid: hsla(t.bad_solid),
            warn_bg: hsla(t.warn_bg),
            warn_line: hsla(t.warn_line),
            bad_bg: hsla(t.bad_bg),
            bad_line: hsla(t.bad_line),
            redis: hsla(t.redis),
            mongo: hsla(t.mongo),
            vnc: hsla(t.vnc),
            syn_blue: hsla(t.syn_blue),
            syn_cyan: hsla(t.syn_cyan),
            syn_yellow: hsla(t.syn_yellow),
            syn_teal: hsla(t.syn_teal),
            syn_purple: hsla(t.syn_purple),
            tag: t.tag.map(hsla),
        }
    }

    pub fn global(cx: &App) -> &Self {
        cx.global::<Self>()
    }
}

/// Shorthand for views: `palette(cx).ink_2`.
pub fn palette(cx: &App) -> &Palette {
    Palette::global(cx)
}

/// Register both TermDrop themes with GPUI Kit and switch to `choice`. Call
/// after `gpui_kit::init` and again whenever the setting changes.
///
/// The fonts are the bundled ones ([`super::fonts::register`] runs first), so
/// there is no "if installed" check here.
pub fn install(choice: ThemeChoice, cx: &mut App) {
    let light = theme_config(ThemeChoice::Light, Some(UI_FONT));
    let dark = theme_config(ThemeChoice::Dark, Some(UI_FONT));
    Theme::update(cx, |theme| {
        theme.light_theme = light;
        theme.dark_theme = dark;
    });
    let mode = match choice {
        ThemeChoice::Dark => ThemeMode::Dark,
        ThemeChoice::Light => ThemeMode::Light,
    };
    // `change` reloads the registered config for the mode even when the mode
    // is already current, which is what applies the configs set above.
    Theme::change(mode, None, cx);
    cx.set_global(Palette::from_tokens(tokens(choice)));
    cx.refresh_windows();
}

/// The active UI font family, for views that set text explicitly.
pub fn ui_font(cx: &App) -> SharedString {
    Theme::global(cx).font_family.clone()
}

/// The monospace family for code in the UI: a command, a path, a key
/// fingerprint, file contents. Vue's `font-mono`. Pair it with a text tier
/// (`text_xs`, `common::text_2xs`); it sets only the family.
pub fn mono_font(cx: &App) -> SharedString {
    Theme::global(cx).mono_font_family.clone()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_brand_accent_is_3b9eff_in_dark() {
        assert_eq!(hex(DARK.accent), "#3b9eff");
    }

    /// Every tag hue clears 3:1 against the sidebar surface in both themes,
    /// the rule `tokens.css` states for them.
    #[test]
    fn group_rail_hues_clear_three_to_one_on_the_surface() {
        fn lum([r, g, b]: Rgb) -> f64 {
            let c = |v: u8| {
                let v = f64::from(v) / 255.;
                if v <= 0.03928 {
                    v / 12.92
                } else {
                    ((v + 0.055) / 1.055).powf(2.4)
                }
            };
            0.2126 * c(r) + 0.7152 * c(g) + 0.0722 * c(b)
        }
        for t in [&LIGHT, &DARK] {
            for tag in t.tag {
                let (a, b) = (lum(tag), lum(t.surface));
                let ratio = (a.max(b) + 0.05) / (a.min(b) + 0.05);
                assert!(ratio >= 3.0, "{tag:?} on {:?}: {ratio:.2}", t.surface);
            }
        }
    }

    #[test]
    fn both_configs_parse_as_gpui_kit_themes() {
        for choice in [ThemeChoice::Light, ThemeChoice::Dark] {
            let config = theme_config(choice, Some(UI_FONT));
            assert_eq!(config.mode.is_dark(), choice == ThemeChoice::Dark);
            assert_eq!(config.font_family.as_deref(), Some(UI_FONT));
            assert_eq!(config.mono_font_family.as_deref(), Some(MONO_FONT));
        }
    }

    #[test]
    fn filled_controls_never_use_the_bright_tone() {
        // `bg-accent text-white` measured 2.79:1 in dark; fills take -solid.
        let json = theme_config_json(ThemeChoice::Dark, None);
        assert_eq!(json["colors"]["primary.background"], hex(DARK.accent_solid));
        assert_ne!(json["colors"]["primary.background"], hex(DARK.accent));
        // Kit buttons read their own keys; unset, a danger button was a tint
        // with a red label.
        assert_eq!(
            json["colors"]["button.danger.background"],
            hex(DARK.bad_solid)
        );
        assert_eq!(json["colors"]["button.danger.foreground"], "#ffffff");
    }
}
