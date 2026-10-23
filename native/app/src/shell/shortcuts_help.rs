//! The keyboard shortcuts dialog. Port of `ShortcutsHelp.vue`: the same
//! sections in the same order, each row a label and a key chip, and the
//! remote-desktop note about Ctrl+Alt+Shift. Opened from the tab bar's
//! keyboard button and by Ctrl+Shift+?.
//!
//! It lists what this app actually binds, which is not quite the Vue list:
//! the terminal has no Select all, find bar or right-click menu yet, so those
//! rows are left out until they exist, and the native-only chords (Ctrl+,
//! Ctrl+Alt+B, Ctrl+PageUp/PageDown, Shift+PageUp/PageDown/Home/End in the
//! scrollback, Ctrl-click on a link, paste) are listed where they belong.

use gpui_kit::component::WindowExt as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::{h_flex, v_flex};
use gpui_kit::{
    App, FontWeight, InteractiveElement as _, IntoElement, ParentElement, SharedString,
    StatefulInteractiveElement as _, Styled, Window, div, px,
};

use crate::panels::common::text_2xs;
use crate::ui::theme::{mono_font, palette};

/// One row: the chord as the chip shows it, and what it does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Shortcut {
    pub keys: &'static str,
    pub label: &'static str,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Section {
    pub title: &'static str,
    pub shortcuts: Vec<Shortcut>,
}

const fn s(keys: &'static str, label: &'static str) -> Shortcut {
    Shortcut { keys, label }
}

/// The note under the remote-desktop section.
pub const VNC_NOTE: &str =
    "A remote desktop takes every keystroke, including the shortcuts above — so";
pub const VNC_NOTE_KEYS: &str = "Ctrl + Alt + Shift";
pub const VNC_NOTE_END: &str = "is the way back out.";

/// Every section, in `ShortcutsHelp.vue`'s order. `mac` swaps the terminal's
/// Ctrl chords for the Cmd ones it binds there.
pub fn sections(mac: bool) -> Vec<Section> {
    let (copy, paste, link) = if mac {
        ("Cmd + C", "Cmd + V", "Cmd + Click")
    } else {
        ("Ctrl + Shift + C", "Ctrl + Shift + V", "Ctrl + Click")
    };
    vec![
        Section {
            title: "Terminal",
            shortcuts: vec![
                s(copy, "Copy selection"),
                s(paste, "Paste"),
                s("Shift + PageUp / PageDown", "Scroll back a page"),
                s("Shift + Home / End", "Top / bottom of the scrollback"),
                s(link, "Open a link"),
            ],
        },
        Section {
            title: "Tabs",
            shortcuts: vec![
                s("Ctrl + Tab", "Next tab"),
                s("Ctrl + Shift + Tab", "Previous tab"),
                s("Ctrl + PageDown / PageUp", "Next / previous tab"),
                s("Ctrl + W", "Close active tab"),
                s("Ctrl + Shift + T", "New local terminal"),
                s("Ctrl + Alt + B", "Show or hide the side panel"),
                s("Ctrl + ,", "Settings"),
                s("Ctrl + Shift + ?", "Keyboard shortcuts"),
            ],
        },
        Section {
            title: "Host List",
            shortcuts: vec![
                s("↑ / ↓", "Move through hosts"),
                s("Home / End", "First / last host"),
                s("PageUp / PageDown", "Move a page at a time"),
                s("Enter", "Connect to the focused host"),
                s("Delete", "Delete the focused host"),
            ],
        },
        Section {
            title: "SFTP",
            shortcuts: vec![
                s("Double-click", "Open folder"),
                s("Right-click", "File actions"),
                s("Drag & drop", "Upload files"),
            ],
        },
        Section {
            title: "Remote desktop",
            shortcuts: vec![
                s("Ctrl + Alt + Shift", "Release the keyboard"),
                s("Click", "Focus the remote desktop"),
                s("Scroll", "Wheel through to the remote"),
            ],
        },
    ]
}

fn kbd(keys: impl Into<SharedString>, cx: &App) -> impl IntoElement {
    let p = *palette(cx);
    div()
        .flex_none()
        .px(px(6.))
        .py(px(2.))
        .rounded_sm()
        .bg(p.input)
        .border_1()
        .border_color(p.line)
        .text_xs()
        .text_color(p.ink)
        .font_family(mono_font(cx))
        .child(keys.into())
}

fn render_sections(cx: &App) -> impl IntoElement {
    let p = *palette(cx);
    let mut body = v_flex().gap_4();
    for section in sections(cfg!(target_os = "macos")) {
        let mut rows = v_flex().gap_1();
        for sc in &section.shortcuts {
            rows = rows.child(
                h_flex()
                    .justify_between()
                    .gap_4()
                    .text_sm()
                    .child(div().text_color(p.ink).child(sc.label))
                    .child(kbd(sc.keys, cx)),
            );
        }
        let mut block = v_flex()
            .child(
                text_2xs(div())
                    .mb_2()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(p.ink_2)
                    .child(section.title.to_uppercase()),
            )
            .child(rows);
        if section.title == "Remote desktop" {
            block = block.child(
                text_2xs(h_flex())
                    .mt_2()
                    .flex_wrap()
                    .gap_1()
                    .text_color(p.ink_3)
                    .child(VNC_NOTE)
                    .child(kbd(VNC_NOTE_KEYS, cx))
                    .child(VNC_NOTE_END),
            );
        }
        body = body.child(block);
    }
    body
}

/// Open the dialog. Escape, the close icon and Close all dismiss it.
pub fn open_shortcuts_help(window: &mut Window, cx: &mut App) {
    window.open_dialog(cx, move |dialog, _, cx| {
        let p = *palette(cx);
        dialog
            .title("Keyboard Shortcuts")
            .w(px(448.))
            .child(
                div()
                    .id("shortcuts-body")
                    .max_h(px(440.))
                    .overflow_y_scroll()
                    .pr_1()
                    .child(render_sections(cx)),
            )
            .footer(
                h_flex()
                    .w_full()
                    .justify_end()
                    .pt_3()
                    .border_t_1()
                    .border_color(p.line)
                    .child(
                        Button::new("shortcuts-close")
                            .primary()
                            .label("Close")
                            .on_click(|_, window, cx| window.close_dialog(cx)),
                    ),
            )
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sections_are_in_the_vue_order() {
        let titles: Vec<_> = sections(false).iter().map(|s| s.title).collect();
        assert_eq!(
            titles,
            ["Terminal", "Tabs", "Host List", "SFTP", "Remote desktop"]
        );
        assert!(sections(false).iter().all(|s| !s.shortcuts.is_empty()));
    }

    #[test]
    fn the_vue_rows_that_exist_here_keep_their_wording() {
        let all: Vec<Shortcut> = sections(false)
            .into_iter()
            .flat_map(|s| s.shortcuts)
            .collect();
        for (keys, label) in [
            ("Ctrl + Shift + C", "Copy selection"),
            ("Ctrl + Tab", "Next tab"),
            ("Ctrl + Shift + Tab", "Previous tab"),
            ("Ctrl + W", "Close active tab"),
            ("Ctrl + Shift + T", "New local terminal"),
            ("↑ / ↓", "Move through hosts"),
            ("Home / End", "First / last host"),
            ("PageUp / PageDown", "Move a page at a time"),
            ("Enter", "Connect to the focused host"),
            ("Delete", "Delete the focused host"),
            ("Double-click", "Open folder"),
            ("Right-click", "File actions"),
            ("Drag & drop", "Upload files"),
            ("Ctrl + Alt + Shift", "Release the keyboard"),
            ("Click", "Focus the remote desktop"),
            ("Scroll", "Wheel through to the remote"),
        ] {
            assert!(all.contains(&s(keys, label)), "{keys} / {label}");
        }
    }

    #[test]
    fn nothing_unbound_is_advertised() {
        // No find bar, select-all or terminal context menu yet.
        let terminal = &sections(false)[0];
        for keys in ["Ctrl + F", "Ctrl + Shift + A", "Esc", "Right-click"] {
            assert!(
                terminal.shortcuts.iter().all(|s| s.keys != keys),
                "{keys} is listed but not bound"
            );
        }
    }

    #[test]
    fn mac_uses_cmd_in_the_terminal() {
        let terminal = &sections(true)[0];
        assert_eq!(terminal.shortcuts[0], s("Cmd + C", "Copy selection"));
        assert_eq!(terminal.shortcuts[4], s("Cmd + Click", "Open a link"));
    }
}
