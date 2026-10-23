//! Asking about a host key. Port of `HostKeyDialog.vue` (first contact) and
//! `HostKeyMismatchDialog.vue` (changed or revoked), as one dialog over one
//! data type.
//!
//! Load-bearing, as in the Tauri app (CLAUDE.md, "Host key verification"):
//!
//! - **An acceptance is the fingerprint, not a boolean.** Accepting answers
//!   with the exact fingerprint string the dialog showed, and the backend
//!   requires it to match the key presented on the retry.
//! - **A mismatch or a revoked key has no accept path.** Its buttons are Copy
//!   details and Close; that is the complete list. The moment "connect
//!   anyway" is reachable from the warning, the warning is decoration.
//! - **A new key type is not a changed identity.** `UnknownForThisType` is a
//!   first-trust decision and is worded as one.
//! - There is deliberately no "don't ask again" and no "trust every host in
//!   this group". The dialog can trust exactly the key it shows, and nothing
//!   is bound to Enter, so a stray keypress cannot trust a host.
//!
//! Wired to the connect path in the integration step; this is the view and
//! its data.

use std::cell::Cell;
use std::rc::Rc;

use gpui_kit::assets::IconName;
use gpui_kit::component::WindowExt as _;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::{Icon, h_flex, v_flex};
use gpui_kit::{
    App, ClipboardItem, FontWeight, IntoElement, ParentElement, SharedString, Styled, Window, div,
    px,
};

use super::icon_button::IconButton;
use super::theme::palette;
use super::toast::{ToastKind, toast};

/// What the trust store concluded about the offered key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostKeyVerdict {
    /// Nothing on file for this host.
    Unknown,
    /// Keys on file, but none of this type. Still a first-trust decision.
    UnknownForThisType,
    /// A stored key of this type differs.
    Mismatch,
    /// The key is listed as revoked.
    Revoked,
}

impl HostKeyVerdict {
    pub fn can_accept(self) -> bool {
        matches!(self, Self::Unknown | Self::UnknownForThisType)
    }
}

/// Which connection asked: the host being opened, or the jump host in front
/// of it. A jumped host can need two answers, one per hop.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Hop {
    #[default]
    Target,
    Jump,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostKeyPrompt {
    pub verdict: HostKeyVerdict,
    pub hop: Hop,
    pub host: String,
    pub port: u16,
    pub fingerprint: String,
    pub key_type: String,
    /// The backend's explanation of a refusal, shown verbatim for a mismatch
    /// or a revoked key. Empty for the first-trust cases.
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HostKeyResponse {
    /// Trust exactly this fingerprint: the string the dialog showed.
    Accept {
        fingerprint: String,
    },
    Reject,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostKeyButton {
    Cancel,
    Trust,
    CopyDetails,
    Close,
}

impl HostKeyPrompt {
    /// The dialog's buttons, in order. Pure so the "no accept path" rule is
    /// tested rather than trusted.
    pub fn buttons(&self) -> &'static [HostKeyButton] {
        if self.verdict.can_accept() {
            &[HostKeyButton::Cancel, HostKeyButton::Trust]
        } else {
            &[HostKeyButton::CopyDetails, HostKeyButton::Close]
        }
    }

    /// The response the Trust button sends, or `None` when there is no Trust
    /// button for this verdict.
    pub fn accept_response(&self) -> Option<HostKeyResponse> {
        self.verdict.can_accept().then(|| HostKeyResponse::Accept {
            fingerprint: self.fingerprint.clone(),
        })
    }

    /// The out-of-band check, spelled out so it is something people might
    /// actually run.
    pub fn verify_command(&self) -> String {
        format!(
            "ssh-keyscan -p {} -t {} {} | ssh-keygen -lf -",
            self.port, self.key_type, self.host
        )
    }

    fn title(&self) -> &'static str {
        match self.verdict {
            HostKeyVerdict::Unknown => "Trust this host?",
            HostKeyVerdict::UnknownForThisType => "Trust this host's new key type?",
            HostKeyVerdict::Mismatch => "Host key verification failed",
            HostKeyVerdict::Revoked => "Host key is revoked",
        }
    }
}

fn copy(text: String, window: &mut Window, cx: &mut App) {
    cx.write_to_clipboard(ClipboardItem::new_string(text));
    toast(window, cx, "Copied", ToastKind::Success);
}

/// Open the dialog. `on_respond` is called exactly once: with `Accept` from
/// the Trust button, and with `Reject` from every other way out.
pub fn open_host_key_dialog(
    window: &mut Window,
    cx: &mut App,
    prompt: HostKeyPrompt,
    on_respond: impl Fn(HostKeyResponse, &mut Window, &mut App) + 'static,
) {
    let answered = Rc::new(Cell::new(false));
    let on_respond = Rc::new(on_respond);
    let respond = move |response: HostKeyResponse, window: &mut Window, cx: &mut App| {
        if answered.replace(true) {
            return;
        }
        window.close_dialog(cx);
        on_respond(response, window, cx);
    };
    let respond = Rc::new(respond);

    window.open_dialog(cx, move |dialog, _, cx| {
        let p = *palette(cx);
        let mono = crate::ui::theme::mono_font(cx);
        let prompt = prompt.clone();
        let place = format!("{}:{}", prompt.host, prompt.port);
        let refused = !prompt.verdict.can_accept();

        let mut body = v_flex().gap_3().text_sm();
        if refused {
            body = body
                .child(
                    h_flex()
                        .gap_2()
                        .items_start()
                        .child(Icon::new(IconName::ShieldAlert).text_color(p.bad))
                        .child(div().text_color(p.ink_2).child(
                            "TermDrop did not connect, and your credentials were not sent.",
                        )),
                )
                .child(
                    div()
                        .p_3()
                        .rounded_sm()
                        .border_1()
                        .border_color(p.bad_line)
                        .bg(p.bad_bg)
                        .text_xs()
                        .font_family(mono.clone())
                        .text_color(p.ink)
                        .child(prompt.message.clone()),
                );
        } else {
            let lead = match prompt.verdict {
                HostKeyVerdict::UnknownForThisType => format!(
                    "TermDrop knows other keys for {place}, but not a {} key. A new key type is \
                     not a changed key; check the fingerprint below against the server before \
                     trusting it.",
                    prompt.key_type
                ),
                _ => format!(
                    "TermDrop has not connected to {place} before. Check the fingerprint below \
                     against the server before trusting it."
                ),
            };
            body = body.child(div().text_color(p.ink_2).child(lead));
            if prompt.hop == Hop::Jump {
                body = body.child(div().text_color(p.ink_2).child(
                    "This is the jump host the connection goes through. The host behind it is \
                     checked separately.",
                ));
            }
            let fingerprint = prompt.fingerprint.clone();
            let command = prompt.verify_command();
            let command_copy = command.clone();
            body = body
                .child(
                    v_flex()
                        .gap_1()
                        .p_3()
                        .rounded_sm()
                        .border_1()
                        .border_color(p.line)
                        .bg(p.input)
                        .child(
                            h_flex()
                                .justify_between()
                                .child(div().text_xs().text_color(p.ink_2).child("Fingerprint"))
                                .child(
                                    div()
                                        .text_xs()
                                        .text_color(p.ink_3)
                                        .child(prompt.key_type.clone()),
                                ),
                        )
                        .child(
                            h_flex()
                                .gap_2()
                                .child(
                                    div()
                                        .flex_1()
                                        .text_xs()
                                        .font_family(mono.clone())
                                        .text_color(p.ink)
                                        .child(prompt.fingerprint.clone()),
                                )
                                .child(
                                    IconButton::new(
                                        "copy-fingerprint",
                                        IconName::Copy,
                                        "Copy fingerprint",
                                    )
                                    .on_click(move |_, w, cx| copy(fingerprint.clone(), w, cx)),
                                ),
                        ),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(p.ink_2)
                        .child("Verify it from a machine you trust:"),
                )
                .child(
                    h_flex()
                        .gap_2()
                        .child(
                            div()
                                .flex_1()
                                .px_2()
                                .py_1()
                                .rounded_sm()
                                .bg(p.input)
                                .text_xs()
                                .font_family(mono.clone())
                                .text_color(p.ink_2)
                                .child(command),
                        )
                        .child(
                            IconButton::new("copy-verify", IconName::Copy, "Copy command")
                                .on_click(move |_, w, cx| copy(command_copy.clone(), w, cx)),
                        ),
                );
        }

        let mut footer = h_flex().justify_end().gap_2();
        for button in prompt.buttons() {
            let respond = respond.clone();
            footer = footer.child(match button {
                HostKeyButton::Cancel => Button::new("hostkey-cancel")
                    .ghost()
                    .label("Cancel")
                    .on_click(move |_, w, cx| respond(HostKeyResponse::Reject, w, cx))
                    .into_any_element(),
                HostKeyButton::Trust => {
                    let response = prompt
                        .accept_response()
                        .expect("Trust is only offered for an acceptable verdict");
                    Button::new("hostkey-trust")
                        .primary()
                        .label("Trust this host")
                        .on_click(move |_, w, cx| respond(response.clone(), w, cx))
                        .into_any_element()
                }
                HostKeyButton::CopyDetails => {
                    let message = prompt.message.clone();
                    Button::new("hostkey-copy")
                        .ghost()
                        .label("Copy details")
                        .on_click(move |_, w, cx| copy(message.clone(), w, cx))
                        .into_any_element()
                }
                HostKeyButton::Close => Button::new("hostkey-close")
                    .danger()
                    .label("Close")
                    .on_click(move |_, w, cx| respond(HostKeyResponse::Reject, w, cx))
                    .into_any_element(),
            });
        }

        let title = div()
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(if refused { p.bad } else { p.ink })
            .child(SharedString::from(prompt.title()));
        let respond = respond.clone();
        dialog
            .title(title)
            .w(px(if refused { 544. } else { 480. }))
            .child(body)
            .footer(footer)
            .on_close(move |_, w, cx| respond(HostKeyResponse::Reject, w, cx))
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn prompt(verdict: HostKeyVerdict) -> HostKeyPrompt {
        HostKeyPrompt {
            verdict,
            hop: Hop::Target,
            host: "bastion.example.com".into(),
            port: 2222,
            fingerprint: "SHA256:abc+/def=".into(),
            key_type: "ssh-ed25519".into(),
            message: "The key for bastion.example.com changed.".into(),
        }
    }

    #[test]
    fn accepting_returns_the_exact_fingerprint_shown() {
        for v in [HostKeyVerdict::Unknown, HostKeyVerdict::UnknownForThisType] {
            assert_eq!(
                prompt(v).accept_response(),
                Some(HostKeyResponse::Accept {
                    fingerprint: "SHA256:abc+/def=".into()
                })
            );
            assert!(prompt(v).buttons().contains(&HostKeyButton::Trust));
        }
    }

    #[test]
    fn a_mismatch_or_revoked_key_has_no_accept_path() {
        for v in [HostKeyVerdict::Mismatch, HostKeyVerdict::Revoked] {
            assert_eq!(prompt(v).accept_response(), None);
            assert!(!prompt(v).buttons().contains(&HostKeyButton::Trust));
            assert_eq!(
                prompt(v).buttons(),
                [HostKeyButton::CopyDetails, HostKeyButton::Close]
            );
        }
    }

    #[test]
    fn a_new_key_type_is_not_worded_as_a_changed_key() {
        assert_ne!(
            prompt(HostKeyVerdict::UnknownForThisType).title(),
            prompt(HostKeyVerdict::Mismatch).title()
        );
    }

    #[test]
    fn the_verify_command_names_port_type_and_host() {
        assert_eq!(
            prompt(HostKeyVerdict::Unknown).verify_command(),
            "ssh-keyscan -p 2222 -t ssh-ed25519 bastion.example.com | ssh-keygen -lf -"
        );
    }
}
