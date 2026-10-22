//! Terminal colours: the 16 ANSI colours from the theme, the derived 256-colour
//! table, and the resolution of a cell's colour into something GPUI can paint.

use alacritty_terminal::term::color::Colors;
use alacritty_terminal::vte::ansi::{Color, NamedColor, Rgb};
use gpui_kit::{Hsla, Rgba};

/// The colours a terminal paints with, as `0xRRGGBB`.
///
/// These are content colours, kept apart from the UI tokens, exactly as
/// `src/themes/index.js` keeps them in the Tauri app.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Palette {
    pub background: u32,
    pub foreground: u32,
    pub cursor: u32,
    pub selection_background: u32,
    /// black, red, green, yellow, blue, magenta, cyan, white, then the bright
    /// variants in the same order.
    pub ansi: [u32; 16],
}

impl Palette {
    /// TermDrop's dark terminal theme (`DARK` in `src/themes/index.js`).
    pub fn dark() -> Self {
        Self {
            background: 0x0F1116,
            foreground: 0xD4D8E0,
            cursor: 0xD4D8E0,
            selection_background: 0x1B3A5C,
            ansi: [
                0x0F1116, 0xF47067, 0x8DDB8C, 0xE3C08B, 0x7FB6EC, 0xD3A2D8, 0x6FD3BE, 0xD4D8E0,
                0x8791A0, 0xFF8A8A, 0xAFE1AE, 0xF0D9AE, 0xA5CDF5, 0xE3BCE6, 0x96E2D2, 0xEDEFF3,
            ],
        }
    }

    /// TermDrop's light terminal theme (`LIGHT` in `src/themes/index.js`).
    pub fn light() -> Self {
        Self {
            background: 0xFFFFFF,
            foreground: 0x1B222C,
            cursor: 0x1B222C,
            selection_background: 0xCFE0F5,
            ansi: [
                0x1B222C, 0xB32017, 0x0F5F33, 0x7A4F00, 0x1461C4, 0x7A3E82, 0x136B5C, 0x556070,
                0x5C6675, 0x8F1912, 0x0B4D29, 0x5E3D00, 0x0F4E9E, 0x5F3066, 0x0E5347, 0x141A22,
            ],
        }
    }

    /// The colour at `index` in the xterm 256-colour table: the 16 theme
    /// colours, then the 6x6x6 cube, then the 24-step grey ramp.
    pub fn indexed(&self, index: u8) -> Rgb {
        match index {
            0..=15 => rgb(self.ansi[index as usize]),
            16..=231 => {
                const LEVELS: [u8; 6] = [0, 95, 135, 175, 215, 255];
                let i = index - 16;
                Rgb {
                    r: LEVELS[(i / 36) as usize],
                    g: LEVELS[((i / 6) % 6) as usize],
                    b: LEVELS[(i % 6) as usize],
                }
            }
            232..=255 => {
                let v = 8 + 10 * (index - 232);
                Rgb { r: v, g: v, b: v }
            }
        }
    }

    /// Resolves a named colour, before any OSC override.
    fn named(&self, color: NamedColor) -> Rgb {
        match color {
            NamedColor::Foreground | NamedColor::BrightForeground => rgb(self.foreground),
            NamedColor::Background => rgb(self.background),
            NamedColor::Cursor => rgb(self.cursor),
            NamedColor::DimForeground => dim(rgb(self.foreground)),
            NamedColor::DimBlack
            | NamedColor::DimRed
            | NamedColor::DimGreen
            | NamedColor::DimYellow
            | NamedColor::DimBlue
            | NamedColor::DimMagenta
            | NamedColor::DimCyan
            | NamedColor::DimWhite => {
                dim(self.indexed((color as usize - NamedColor::DimBlack as usize) as u8))
            }
            // Black..BrightWhite are 0..16.
            other => self.indexed(other as usize as u8),
        }
    }

    /// The colour a cell attribute stands for. `overrides` are the colours a
    /// program set with OSC 4/10/11/12, which win over the theme.
    pub fn resolve(&self, color: Color, overrides: &Colors) -> Rgb {
        match color {
            Color::Spec(rgb) => rgb,
            Color::Indexed(i) => overrides[i as usize].unwrap_or_else(|| self.indexed(i)),
            Color::Named(named) => overrides[named as usize].unwrap_or_else(|| self.named(named)),
        }
    }
}

impl Default for Palette {
    fn default() -> Self {
        Self::dark()
    }
}

pub(crate) fn rgb(hex: u32) -> Rgb {
    Rgb {
        r: (hex >> 16) as u8,
        g: (hex >> 8) as u8,
        b: hex as u8,
    }
}

/// Faint text (SGR 2) at two thirds brightness, as alacritty and xterm.js do.
pub(crate) fn dim(c: Rgb) -> Rgb {
    let f = |v: u8| ((v as u16 * 2) / 3) as u8;
    Rgb {
        r: f(c.r),
        g: f(c.g),
        b: f(c.b),
    }
}

pub(crate) fn hsla(c: Rgb) -> Hsla {
    Rgba {
        r: c.r as f32 / 255.0,
        g: c.g as f32 / 255.0,
        b: c.b as f32 / 255.0,
        a: 1.0,
    }
    .into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cube_and_greys_follow_xterm() {
        let p = Palette::dark();
        assert_eq!(p.indexed(16), Rgb { r: 0, g: 0, b: 0 });
        assert_eq!(p.indexed(196), Rgb { r: 255, g: 0, b: 0 });
        assert_eq!(
            p.indexed(231),
            Rgb {
                r: 255,
                g: 255,
                b: 255
            }
        );
        assert_eq!(p.indexed(232), Rgb { r: 8, g: 8, b: 8 });
        assert_eq!(
            p.indexed(255),
            Rgb {
                r: 238,
                g: 238,
                b: 238
            }
        );
    }

    #[test]
    fn named_colours_come_from_the_theme() {
        let p = Palette::dark();
        let none = Colors::default();
        assert_eq!(
            p.resolve(Color::Named(NamedColor::Red), &none),
            rgb(0xF47067)
        );
        assert_eq!(
            p.resolve(Color::Named(NamedColor::BrightWhite), &none),
            rgb(0xEDEFF3)
        );
        assert_eq!(
            p.resolve(Color::Named(NamedColor::Background), &none),
            rgb(0x0F1116)
        );
        assert_eq!(p.resolve(Color::Indexed(4), &none), rgb(0x7FB6EC));
    }

    #[test]
    fn dim_colours_are_the_normal_ones_faded() {
        let p = Palette::dark();
        let none = Colors::default();
        assert_eq!(
            p.resolve(Color::Named(NamedColor::DimRed), &none),
            dim(rgb(0xF47067))
        );
    }

    #[test]
    fn osc_overrides_win() {
        let p = Palette::dark();
        let mut colors = Colors::default();
        colors[NamedColor::Foreground] = Some(Rgb { r: 1, g: 2, b: 3 });
        assert_eq!(
            p.resolve(Color::Named(NamedColor::Foreground), &colors),
            Rgb { r: 1, g: 2, b: 3 }
        );
    }
}
