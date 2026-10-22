//! Mouse reports for programs that asked for them (DECSET 1000/1002/1003),
//! in the SGR (1006), UTF-8 (1005) or legacy X10 encoding.

/// Which button a report is about, as the xterm button number.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReportButton {
    Left = 0,
    Middle = 1,
    Right = 2,
    /// Motion with no button held (any-event tracking).
    None = 3,
    WheelUp = 64,
    WheelDown = 65,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReportKind {
    Press,
    Release,
    Motion,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ReportModifiers {
    pub shift: bool,
    pub alt: bool,
    pub control: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ReportEncoding {
    pub sgr: bool,
    pub utf8: bool,
}

/// The report for one mouse event at the 0-based cell `col`, `row`, or `None`
/// when the legacy encoding cannot express the position.
pub fn mouse_report(
    button: ReportButton,
    kind: ReportKind,
    mods: ReportModifiers,
    col: usize,
    row: usize,
    enc: ReportEncoding,
) -> Option<Vec<u8>> {
    let mut code = button as u32;
    if kind == ReportKind::Motion {
        code += 32;
    }
    code += (mods.shift as u32) * 4 + (mods.alt as u32) * 8 + (mods.control as u32) * 16;

    if enc.sgr {
        // SGR keeps the button on release and marks it with a final `m`.
        let fin = if kind == ReportKind::Release {
            'm'
        } else {
            'M'
        };
        return Some(format!("\x1b[<{code};{};{}{fin}", col + 1, row + 1).into_bytes());
    }

    // The legacy encodings cannot say which button was released.
    if kind == ReportKind::Release {
        code = 3 + (code & !3 & !64);
    }
    let mut out = b"\x1b[M".to_vec();
    out.push((32 + code) as u8);
    for v in [col, row] {
        let v = v as u32 + 1 + 32;
        if enc.utf8 {
            // 1005 encodes coordinates as UTF-8 characters, up to 2015.
            if v > 0x7ff {
                return None;
            }
            let mut buf = [0u8; 4];
            out.extend_from_slice(char::from_u32(v)?.encode_utf8(&mut buf).as_bytes());
        } else {
            if v > 255 {
                return None;
            }
            out.push(v as u8);
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SGR: ReportEncoding = ReportEncoding {
        sgr: true,
        utf8: false,
    };
    const X10: ReportEncoding = ReportEncoding {
        sgr: false,
        utf8: false,
    };
    const NONE: ReportModifiers = ReportModifiers {
        shift: false,
        alt: false,
        control: false,
    };

    #[test]
    fn sgr_reports() {
        let r = |b, k, c, r| mouse_report(b, k, NONE, c, r, SGR).unwrap();
        assert_eq!(
            r(ReportButton::Left, ReportKind::Press, 0, 0),
            b"\x1b[<0;1;1M"
        );
        assert_eq!(
            r(ReportButton::Left, ReportKind::Release, 4, 9),
            b"\x1b[<0;5;10m"
        );
        assert_eq!(
            r(ReportButton::Left, ReportKind::Motion, 1, 1),
            b"\x1b[<32;2;2M"
        );
        assert_eq!(
            r(ReportButton::WheelDown, ReportKind::Press, 0, 0),
            b"\x1b[<65;1;1M"
        );
        let ctrl = ReportModifiers {
            control: true,
            ..NONE
        };
        assert_eq!(
            mouse_report(ReportButton::Right, ReportKind::Press, ctrl, 0, 0, SGR).unwrap(),
            b"\x1b[<18;1;1M"
        );
    }

    #[test]
    fn legacy_reports() {
        let press = mouse_report(ReportButton::Left, ReportKind::Press, NONE, 0, 0, X10).unwrap();
        assert_eq!(press, b"\x1b[M\x20\x21\x21");
        let release =
            mouse_report(ReportButton::Right, ReportKind::Release, NONE, 2, 3, X10).unwrap();
        assert_eq!(release, b"\x1b[M\x23\x23\x24");
        // Past column 223 the legacy encoding has no byte for it.
        assert!(mouse_report(ReportButton::Left, ReportKind::Press, NONE, 300, 0, X10).is_none());
        let utf8 = ReportEncoding {
            sgr: false,
            utf8: true,
        };
        let far = mouse_report(ReportButton::Left, ReportKind::Press, NONE, 300, 0, utf8).unwrap();
        assert_eq!(&far[4..6], "\u{14d}".as_bytes());
    }
}
