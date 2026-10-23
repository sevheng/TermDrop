//! Layout preferences and settings, in the encodings the Tauri app writes.
//!
//! Same keys and value formats as `src/composables/usePrefs.js` (layout) and
//! `stores/connection.js` (`theme`, `font_size`), so both apps share one
//! `settings` table while they run side by side. A value that does not decode
//! is ignored in favour of the default, as `usePrefs.js`'s `decode` does.

use serde::{Deserialize, Serialize};

use crate::backend::HostStore;

pub const SIDEBAR_WIDTH: &str = "sidebar-width";
pub const SFTP_WIDTH: &str = "sftp-width";
pub const HOST_VIEW_MODE: &str = "host-view-mode";
pub const HOST_COLLAPSED_GROUPS: &str = "host-collapsed-groups";
pub const HOST_CUSTOM_GROUPS: &str = "host-custom-groups";
pub const THEME: &str = "theme";
pub const FONT_SIZE: &str = "font_size";
/// Stored and shown by the settings dialog. Nothing reads it yet, in either
/// app: SFTP downloads always go to the Downloads folder.
pub const DOWNLOAD_PATH: &str = "download_path";

/// The drag limits MainWindow.vue clamps to.
pub const SIDEBAR_RANGE: (f32, f32) = (150.0, 400.0);
pub const SFTP_RANGE: (f32, f32) = (180.0, 420.0);
/// What a stored font size is clamped to when read.
pub const FONT_SIZE_RANGE: (u16, u16) = (8, 32);
/// The settings dialog's slider: `SettingsPanel.vue`'s range input.
pub const FONT_SIZE_SLIDER: (u16, u16) = (10, 24);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ViewMode {
    #[default]
    Grouped,
    Flat,
}

impl ViewMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Grouped => "grouped",
            Self::Flat => "flat",
        }
    }

    fn parse(raw: &str) -> Option<Self> {
        match raw {
            "grouped" => Some(Self::Grouped),
            "flat" => Some(Self::Flat),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ThemeChoice {
    /// The Tauri app's default, kept so an unset setting looks the same in
    /// both apps.
    #[default]
    Dark,
    Light,
}

impl ThemeChoice {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Dark => "dark",
            Self::Light => "light",
        }
    }

    fn parse(raw: &str) -> Option<Self> {
        match raw {
            "dark" => Some(Self::Dark),
            "light" => Some(Self::Light),
            _ => None,
        }
    }
}

/// `parseInt`: leading whitespace, an optional sign, then as many digits as
/// there are. `"240px"` is 240, as it was for the Vue app.
pub fn parse_int(raw: &str) -> Option<i64> {
    let s = raw.trim_start();
    let (sign, digits) = match s.as_bytes().first() {
        Some(b'-') => (-1, &s[1..]),
        Some(b'+') => (1, &s[1..]),
        _ => (1, s),
    };
    let end = digits
        .bytes()
        .position(|b| !b.is_ascii_digit())
        .unwrap_or(digits.len());
    digits[..end].parse::<i64>().ok().map(|n| sign * n)
}

fn parse_list(raw: &str) -> Option<Vec<String>> {
    serde_json::from_str(raw).ok()
}

pub fn encode_list(list: &[String]) -> String {
    serde_json::to_string(list).unwrap_or_else(|_| "[]".into())
}

pub fn clamp_width(w: f32, (lo, hi): (f32, f32)) -> f32 {
    w.clamp(lo, hi)
}

#[derive(Debug, Clone, PartialEq)]
pub struct Prefs {
    pub sidebar_width: f32,
    pub sftp_width: f32,
    pub view_mode: ViewMode,
    pub collapsed_groups: Vec<String>,
    pub custom_groups: Vec<String>,
    pub theme: ThemeChoice,
    pub font_size: u16,
}

impl Default for Prefs {
    fn default() -> Self {
        Self {
            sidebar_width: 220.0,
            sftp_width: 260.0,
            view_mode: ViewMode::Grouped,
            collapsed_groups: Vec::new(),
            custom_groups: Vec::new(),
            theme: ThemeChoice::Dark,
            font_size: 14,
        }
    }
}

impl Prefs {
    /// Decode whatever the store holds, field by field, so one unreadable key
    /// costs only that key.
    pub fn decode(get: impl Fn(&str) -> Option<String>) -> Self {
        let d = Self::default();
        let int = |key| get(key).as_deref().and_then(parse_int);
        Self {
            sidebar_width: int(SIDEBAR_WIDTH)
                .map(|n| clamp_width(n as f32, SIDEBAR_RANGE))
                .unwrap_or(d.sidebar_width),
            sftp_width: int(SFTP_WIDTH)
                .map(|n| clamp_width(n as f32, SFTP_RANGE))
                .unwrap_or(d.sftp_width),
            view_mode: get(HOST_VIEW_MODE)
                .as_deref()
                .and_then(ViewMode::parse)
                .unwrap_or(d.view_mode),
            collapsed_groups: get(HOST_COLLAPSED_GROUPS)
                .as_deref()
                .and_then(parse_list)
                .unwrap_or_default(),
            custom_groups: get(HOST_CUSTOM_GROUPS)
                .as_deref()
                .and_then(parse_list)
                .unwrap_or_default(),
            theme: get(THEME)
                .as_deref()
                .and_then(ThemeChoice::parse)
                .unwrap_or(d.theme),
            font_size: int(FONT_SIZE)
                .and_then(|n| u16::try_from(n).ok())
                .map(|n| n.clamp(FONT_SIZE_RANGE.0, FONT_SIZE_RANGE.1))
                .unwrap_or(d.font_size),
        }
    }

    /// Read every preference from the store. A read error is the same as an
    /// unset key: the app must start even when the settings table cannot be
    /// read.
    pub fn load(store: &dyn HostStore) -> Self {
        Self::decode(|key| store.get_setting(key).ok().flatten())
    }
}

/// What the settings dialog edits until Save: `SettingsPanel.vue`'s three
/// refs. Nothing is written while it is open; the theme is only previewed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SettingsDraft {
    pub font_size: u16,
    pub download_path: String,
    pub theme: ThemeChoice,
}

impl SettingsDraft {
    /// The dialog's opening values. A stored size outside the slider's range
    /// is kept until the slider moves, as Vue's `v-model` on a range input
    /// keeps it: only the thumb is drawn at the end ([`Self::slider_value`]).
    pub fn open(font_size: u16, download_path: Option<String>, theme: ThemeChoice) -> Self {
        Self {
            font_size,
            download_path: download_path.unwrap_or_default(),
            theme,
        }
    }

    /// Where the slider's thumb sits.
    pub fn slider_value(&self) -> u16 {
        self.font_size.clamp(FONT_SIZE_SLIDER.0, FONT_SIZE_SLIDER.1)
    }

    /// The slider's label.
    pub fn font_size_label(&self) -> String {
        format!("Terminal Font Size: {}px", self.font_size)
    }

    /// Every setting Save writes, in `saveSettings`' keys and encodings. All
    /// three are written each time, changed or not, as the Vue store does.
    pub fn saved_values(&self) -> [(&'static str, String); 3] {
        [
            (FONT_SIZE, self.font_size.to_string()),
            (DOWNLOAD_PATH, self.download_path.clone()),
            (THEME, self.theme.as_str().to_string()),
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn the_settings_draft_writes_what_save_settings_writes() {
        let mut d = SettingsDraft::open(14, None, ThemeChoice::Dark);
        assert_eq!(d.font_size_label(), "Terminal Font Size: 14px");
        d.font_size = 18;
        d.download_path = "/tmp/dl".into();
        d.theme = ThemeChoice::Light;
        assert_eq!(
            d.saved_values(),
            [
                ("font_size", "18".to_string()),
                ("download_path", "/tmp/dl".to_string()),
                ("theme", "light".to_string()),
            ]
        );
        // An unset path is saved as the empty string, "use the default".
        let d = SettingsDraft::open(14, None, ThemeChoice::Dark);
        assert_eq!(d.saved_values()[1], ("download_path", String::new()));
    }

    #[test]
    fn the_slider_opens_inside_its_range() {
        let big = SettingsDraft::open(30, None, ThemeChoice::Dark);
        assert_eq!(big.slider_value(), 24);
        // Untouched, Save keeps what was stored.
        assert_eq!(big.saved_values()[0], ("font_size", "30".to_string()));
        assert_eq!(
            SettingsDraft::open(8, None, ThemeChoice::Dark).slider_value(),
            10
        );
        assert_eq!(
            SettingsDraft::open(12, Some("~/dl".into()), ThemeChoice::Dark).download_path,
            "~/dl"
        );
    }

    fn decode(pairs: &[(&str, &str)]) -> Prefs {
        let map: HashMap<String, String> = pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        Prefs::decode(|k| map.get(k).cloned())
    }

    #[test]
    fn defaults_match_the_vue_app() {
        let p = decode(&[]);
        assert_eq!(p.sidebar_width, 220.0);
        assert_eq!(p.sftp_width, 260.0);
        assert_eq!(p.view_mode, ViewMode::Grouped);
        assert_eq!(p.theme, ThemeChoice::Dark);
        assert_eq!(p.font_size, 14);
    }

    #[test]
    fn reads_the_encodings_use_prefs_writes() {
        let p = decode(&[
            (SIDEBAR_WIDTH, "240"),
            (SFTP_WIDTH, "300"),
            (HOST_VIEW_MODE, "flat"),
            (HOST_COLLAPSED_GROUPS, r#"["Production","2"]"#),
            (HOST_CUSTOM_GROUPS, r#"["staging"]"#),
            (THEME, "light"),
            (FONT_SIZE, "16"),
        ]);
        assert_eq!(p.sidebar_width, 240.0);
        assert_eq!(p.sftp_width, 300.0);
        assert_eq!(p.view_mode, ViewMode::Flat);
        assert_eq!(p.collapsed_groups, ["Production", "2"]);
        assert_eq!(p.custom_groups, ["staging"]);
        assert_eq!(p.theme, ThemeChoice::Light);
        assert_eq!(p.font_size, 16);
    }

    #[test]
    fn ignores_values_that_do_not_decode() {
        let p = decode(&[
            (SIDEBAR_WIDTH, "wide"),
            (HOST_VIEW_MODE, "tiles"),
            (HOST_COLLAPSED_GROUPS, "not json"),
            (THEME, "solarized"),
        ]);
        assert_eq!(p, Prefs::default());
    }

    #[test]
    fn clamps_widths_to_the_drag_limits() {
        let p = decode(&[(SIDEBAR_WIDTH, "9000"), (SFTP_WIDTH, "10")]);
        assert_eq!(p.sidebar_width, 400.0);
        assert_eq!(p.sftp_width, 180.0);
    }

    #[test]
    fn parses_like_parse_int() {
        assert_eq!(parse_int("240"), Some(240));
        assert_eq!(parse_int(" 240px"), Some(240));
        assert_eq!(parse_int("-3"), Some(-3));
        assert_eq!(parse_int("px"), None);
    }

    #[test]
    fn lists_round_trip_as_json_arrays() {
        let list = vec!["a".to_string(), "b \"c\"".to_string()];
        assert_eq!(parse_list(&encode_list(&list)), Some(list));
    }
}
