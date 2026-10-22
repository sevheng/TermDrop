//! Connecting an SSH tab: the port of the store's `connect` in
//! `src/stores/connection.js`, with its host-key and password retries.
//!
//! - A host key the user has not seen is asked about with the host key
//!   dialog, at most once per hop (jump host, then target). The acceptance
//!   sent back is the fingerprint the dialog showed, never a yes, and the core
//!   compares it with the key presented on the retry.
//! - A changed or revoked key shows the dialog without an accept path, and
//!   nothing here retries past it.
//! - A password missing from the keyring is asked for, stored, and retried
//!   with, unless the host uses a key.

use termdrop_core::{CoreError, TrustPrompt, crypto, db, state};

use gpui_kit::{Context, Window};

use super::terminal::{Status, TerminalTab};
use crate::ui::dialogs::{PromptOptions, prompt};
use crate::ui::host_key_dialog::{
    Hop, HostKeyPrompt, HostKeyResponse, HostKeyVerdict, open_host_key_dialog,
};

/// What one connect attempt carries: the answers the user has given so far.
#[derive(Debug, Clone, Default)]
pub(super) struct Attempt {
    pub password: Option<String>,
    /// The fingerprint accepted for the target host on this attempt.
    pub accept_target: Option<String>,
    /// The same, for the jump host in front of it.
    pub accept_jump: Option<String>,
}

impl Attempt {
    fn accepted_for(&self, jump: bool) -> bool {
        if jump {
            self.accept_jump.is_some()
        } else {
            self.accept_target.is_some()
        }
    }

    fn accept(mut self, jump: bool, fingerprint: String) -> Self {
        if jump {
            self.accept_jump = Some(fingerprint);
        } else {
            self.accept_target = Some(fingerprint);
        }
        self
    }
}

/// The dialog's view of a refusal. `kind` is `unknown` or `refused`; the
/// payload does not say whether an unknown key is the host's first or only a
/// new type, so both read as first trust, which is what the Vue dialog did.
fn dialog_prompt(p: &TrustPrompt) -> HostKeyPrompt {
    let verdict = if p.can_accept() {
        HostKeyVerdict::Unknown
    } else if p.message.to_lowercase().contains("revoked") {
        HostKeyVerdict::Revoked
    } else {
        HostKeyVerdict::Mismatch
    };
    HostKeyPrompt {
        verdict,
        hop: if p.is_jump() { Hop::Jump } else { Hop::Target },
        host: p.host.clone(),
        port: p.port,
        fingerprint: p.fingerprint.clone(),
        key_type: p.key_type.clone(),
        message: p.message.clone(),
    }
}

/// What to do with a failed connect. Pure, so the "once per hop" and "no
/// accept path" rules are tested rather than trusted.
#[derive(Debug, PartialEq)]
pub(super) enum Next {
    /// Ask about this key; `accept` false means the dialog has no Trust
    /// button and the attempt ends when it closes.
    AskTrust {
        accept: bool,
    },
    AskPassword,
    Fail(String),
}

pub(super) fn next_step(err: &CoreError, attempt: &Attempt, key_auth: bool) -> Next {
    match err {
        CoreError::Trust(p) if !p.can_accept() => Next::AskTrust { accept: false },
        CoreError::Trust(p) if attempt.accepted_for(p.is_jump()) => Next::Fail(format!(
            "The {} offered a different key ({}) after you trusted one. Not connecting.",
            if p.is_jump() { "jump host" } else { "host" },
            p.fingerprint
        )),
        CoreError::Trust(_) => Next::AskTrust { accept: true },
        CoreError::NeedsPassword(_) if !key_auth && attempt.password.is_none() => Next::AskPassword,
        other => Next::Fail(format!("SSH connection failed: {other}")),
    }
}

impl TerminalTab {
    pub(super) fn connect_ssh(
        &mut self,
        host_id: i64,
        attempt: Attempt,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.closed {
            return;
        }
        self.set_status(Status::Connecting, cx);
        self.model
            .update(cx, |m, cx| m.set_connecting_host(Some(host_id), cx));
        let (cols, rows) = *super::terminal::lock(&self.size);
        let services = self.services.clone();
        let a = attempt.clone();
        let task = self.services.rt.spawn(async move {
            services
                .core
                .ssh_connect(
                    services.events(),
                    host_id,
                    a.password,
                    a.accept_target,
                    a.accept_jump,
                    u32::from(cols),
                    u32::from(rows),
                )
                .await
        });
        self.pending = Some(cx.spawn_in(window, async move |this, cx| {
            let result = task
                .await
                .unwrap_or_else(|e| Err(CoreError::Other(format!("connect task failed: {e}"))));
            let _ = cx.update(|window, cx| {
                this.update(cx, |this, cx| {
                    this.ssh_result(host_id, attempt, result, window, cx)
                })
            });
        }));
    }

    fn ssh_result(
        &mut self,
        host_id: i64,
        attempt: Attempt,
        result: Result<String, CoreError>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.pending = None;
        self.model
            .update(cx, |m, cx| m.set_connecting_host(None, cx));
        if self.closed {
            if let Ok(id) = result {
                self.disconnect(id);
            }
            return;
        }
        let err = match result {
            Ok(id) => {
                self.adopt_session(id, cx);
                // The sidebar's "connected 2m ago". A failure here is not the
                // user's problem: the session is up.
                if let Err(e) = state::with_db(&self.services.core, |conn| {
                    db::update_host_last_connected(conn, host_id)
                }) {
                    tracing::warn!("Could not record the connection time: {e}");
                }
                self.model.update(cx, |m, cx| m.reload_hosts(cx));
                return;
            }
            Err(e) => e,
        };
        let key_auth = self
            .services
            .core
            .get_host(host_id)
            .ok()
            .flatten()
            .is_some_and(|h| h.auth_type == "key");
        match (next_step(&err, &attempt, key_auth), err) {
            (Next::AskTrust { accept }, CoreError::Trust(p)) => {
                self.ask_trust(host_id, attempt, *p, accept, window, cx)
            }
            (Next::AskPassword, err) => self.ask_password(host_id, attempt, err, window, cx),
            (Next::Fail(message), _) => self.set_status(Status::Failed(message), cx),
            (Next::AskTrust { .. }, other) => {
                self.set_status(Status::Failed(other.to_string()), cx)
            }
        }
    }

    fn ask_trust(
        &mut self,
        host_id: i64,
        attempt: Attempt,
        p: TrustPrompt,
        accept: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let jump = p.is_jump();
        let refusal = if accept {
            "Host key not trusted. Nothing was sent to the server.".to_string()
        } else {
            format!(
                "Host key verification failed for {}:{}. Nothing was sent to the server.",
                p.host, p.port
            )
        };
        // The tab stays "connecting" while the dialog is up: nothing has
        // failed until the user says no.
        let this = cx.entity().downgrade();
        open_host_key_dialog(
            window,
            cx,
            dialog_prompt(&p),
            move |response, window, cx| {
                let _ = this.update(cx, |this, cx| match response {
                    // `accept` is checked again here, not only by the dialog's
                    // buttons: a refused key must have no path through.
                    HostKeyResponse::Accept { fingerprint } if accept => {
                        let next = attempt.clone().accept(jump, fingerprint);
                        this.connect_ssh(host_id, next, window, cx);
                    }
                    _ => this.set_status(Status::Failed(refusal.clone()), cx),
                });
            },
        );
    }

    fn ask_password(
        &mut self,
        host_id: i64,
        attempt: Attempt,
        err: CoreError,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let failure = format!("SSH connection failed: {err}");
        let this = cx.entity().downgrade();
        prompt(
            window,
            cx,
            PromptOptions::new("Password required")
                .message("Password not found in keyring. Enter password for this host:")
                .password()
                .confirm_label("Connect"),
            move |answer, window, cx| {
                let _ = this.update(cx, |this, cx| match answer.filter(|pw| !pw.is_empty()) {
                    Some(password) => {
                        // Stored before the retry, as the Vue store did, so a
                        // second tab on this host does not ask again.
                        if let Err(e) = crypto::store_password(host_id, &password) {
                            tracing::warn!("Could not store the password: {e}");
                        }
                        let next = Attempt {
                            password: Some(password),
                            ..attempt.clone()
                        };
                        this.connect_ssh(host_id, next, window, cx);
                    }
                    None => this.set_status(Status::Failed(failure.clone()), cx),
                });
            },
        );
    }

    /// Restart an ended session under the same id, SSH or local. A host key
    /// that changed since the tab opened fails the reconnect rather than
    /// asking again; that is the core's rule.
    pub(super) fn reconnect(
        &mut self,
        password: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(session_id) = self.session_id() else {
            return;
        };
        self.set_status(Status::Connecting, cx);
        let services = self.services.clone();
        let id = session_id.clone();
        let pw = password.clone();
        let task = self.services.rt.spawn(async move {
            services
                .core
                .ssh_reconnect(services.events(), &id, None, pw)
                .await
        });
        self.pending = Some(cx.spawn_in(window, async move |this, cx| {
            let result = task
                .await
                .unwrap_or_else(|e| Err(CoreError::Other(format!("reconnect task failed: {e}"))));
            let _ = cx.update(|window, cx| {
                this.update(cx, |this, cx| {
                    this.reconnect_result(session_id, password, result, window, cx)
                })
            });
        }));
    }

    fn reconnect_result(
        &mut self,
        session_id: String,
        password: Option<String>,
        result: Result<(), CoreError>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.pending = None;
        if self.closed {
            return;
        }
        match result {
            Ok(()) => {
                // An SSH reconnect replaces the session's output channel; a
                // local one carries it over, and attaching again is harmless.
                self.attach_output(&session_id, cx);
                self.send_size(&session_id);
            }
            Err(CoreError::NeedsPassword(_)) if password.is_none() => {
                let this = cx.entity().downgrade();
                prompt(
                    window,
                    cx,
                    PromptOptions::new("Password required")
                        .message("Password not found in keyring. Enter password for this host:")
                        .password()
                        .confirm_label("Reconnect"),
                    move |answer, window, cx| {
                        let _ =
                            this.update(cx, |this, cx| match answer.filter(|p| !p.is_empty()) {
                                Some(password) => this.reconnect(Some(password), window, cx),
                                None => this.set_status(Status::Ended(None), cx),
                            });
                    },
                );
            }
            Err(e) => self.set_status(Status::Ended(Some(format!("Reconnect failed: {e}"))), cx),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use termdrop_core::ssh::trust::TRUST_PREFIX;

    fn trust(kind: &str, hop: &str, message: &str) -> CoreError {
        CoreError::from(format!(
            "{TRUST_PREFIX}{}",
            serde_json::json!({
                "kind": kind, "host": "h", "port": 22, "fingerprint": "SHA256:abc",
                "keyType": "ssh-ed25519", "message": message, "hop": hop,
            })
        ))
    }

    #[test]
    fn an_unknown_key_is_asked_about_once_per_hop() {
        let none = Attempt::default();
        assert_eq!(
            next_step(&trust("unknown", "target", ""), &none, false),
            Next::AskTrust { accept: true }
        );
        // The bastion was accepted; the target is still its own question.
        let jumped = none.clone().accept(true, "SHA256:j".into());
        assert_eq!(
            next_step(&trust("unknown", "target", ""), &jumped, false),
            Next::AskTrust { accept: true }
        );
        // Asked already for this hop: never a second dialog.
        assert!(matches!(
            next_step(&trust("unknown", "jump", ""), &jumped, false),
            Next::Fail(_)
        ));
    }

    #[test]
    fn a_refused_key_has_no_accept_path() {
        for attempt in [
            Attempt::default(),
            Attempt::default().accept(false, "SHA256:x".into()),
        ] {
            assert_eq!(
                next_step(&trust("refused", "target", "changed"), &attempt, false),
                Next::AskTrust { accept: false }
            );
        }
        let p = match trust("refused", "target", "This key is REVOKED") {
            CoreError::Trust(p) => p,
            _ => unreachable!(),
        };
        let shown = dialog_prompt(&p);
        assert_eq!(shown.verdict, HostKeyVerdict::Revoked);
        assert!(shown.accept_response().is_none());
    }

    #[test]
    fn the_acceptance_is_the_fingerprint_for_its_hop() {
        let a = Attempt::default().accept(true, "SHA256:jump".into());
        assert_eq!(a.accept_jump.as_deref(), Some("SHA256:jump"));
        assert_eq!(a.accept_target, None);
        let p = match trust("unknown", "jump", "") {
            CoreError::Trust(p) => p,
            _ => unreachable!(),
        };
        let shown = dialog_prompt(&p);
        assert_eq!(shown.hop, Hop::Jump);
        assert_eq!(
            shown.accept_response(),
            Some(HostKeyResponse::Accept {
                fingerprint: "SHA256:abc".into()
            })
        );
    }

    #[test]
    fn a_keyring_miss_asks_once_and_never_for_a_key_host() {
        let miss = CoreError::from("keyring retrieve failed: No matching entry".to_string());
        assert_eq!(
            next_step(&miss, &Attempt::default(), false),
            Next::AskPassword
        );
        assert!(matches!(
            next_step(&miss, &Attempt::default(), true),
            Next::Fail(_)
        ));
        let typed = Attempt {
            password: Some("pw".into()),
            ..Attempt::default()
        };
        assert!(matches!(next_step(&miss, &typed, false), Next::Fail(_)));
    }

    #[test]
    fn other_errors_keep_their_text() {
        let e = CoreError::from("Connection refused".to_string());
        assert_eq!(
            next_step(&e, &Attempt::default(), false),
            Next::Fail("SSH connection failed: Connection refused".into())
        );
    }
}
