//! The fonts the app ships, embedded in the binary and registered with GPUI
//! before the first window, so the UI and the terminal look the same on every
//! machine whether or not the fonts are installed. The Tauri app bundles the
//! same families as woff2 for its webview; GPUI needs TTF.
//!
//! - IBM Plex Sans: the UI ([`UI_FONT`]).
//! - JetBrains Mono: the terminal and code in the UI ([`MONO_FONT`]).
//! - Symbols Nerd Font Mono: icon glyphs in prompts and file listings, the
//!   terminal's fallback ([`termdrop_terminal::NERD_SYMBOLS_FAMILY`]).
//!
//! Licences (SIL OFL 1.1, MIT) sit beside the files in `assets/fonts/`.

use std::borrow::Cow;

use gpui_kit::App;

/// The UI family, as the Vue app's `font-sans`.
pub const UI_FONT: &str = "IBM Plex Sans";
/// The monospace family, as the Vue app's `font-mono` and the terminal.
pub const MONO_FONT: &str = termdrop_terminal::DEFAULT_FONT_FAMILY;

macro_rules! font {
    ($file:literal) => {
        include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/fonts/", $file)).as_slice()
    };
}

/// Every embedded face. Plex's Medium and SemiBold are the weights the UI
/// asks for (`FontWeight::MEDIUM`, `SEMIBOLD`); without the faces they would
/// be synthesised from Regular or snapped to Bold.
const FACES: &[&[u8]] = &[
    font!("IBMPlexSans-Regular.ttf"),
    font!("IBMPlexSans-Medium.ttf"),
    font!("IBMPlexSans-SemiBold.ttf"),
    font!("IBMPlexSans-Bold.ttf"),
    font!("JetBrainsMono-Regular.ttf"),
    font!("JetBrainsMono-Medium.ttf"),
    font!("JetBrainsMono-Bold.ttf"),
    font!("JetBrainsMono-Italic.ttf"),
    font!("JetBrainsMono-BoldItalic.ttf"),
    font!("SymbolsNerdFontMono-Regular.ttf"),
];

/// Register the embedded fonts. Call first thing at startup: before the theme
/// is installed and before any window or terminal resolves a family.
pub fn register(cx: &App) {
    let faces = FACES.iter().map(|f| Cow::Borrowed(*f)).collect();
    if let Err(e) = cx.text_system().add_fonts(faces) {
        // Not fatal: the theme and the terminal still fall back to what is
        // installed.
        tracing::error!("Could not load the bundled fonts: {e:#}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_face_is_a_truetype_file() {
        for face in FACES {
            // sfnt version 1.0: TrueType outlines.
            assert_eq!(&face[..4], &[0, 1, 0, 0]);
        }
    }

    #[test]
    fn the_terminal_falls_back_to_the_bundled_symbols() {
        assert_eq!(
            termdrop_terminal::NERD_SYMBOLS_FAMILY,
            "Symbols Nerd Font Mono"
        );
        assert_eq!(MONO_FONT, "JetBrains Mono");
    }
}
