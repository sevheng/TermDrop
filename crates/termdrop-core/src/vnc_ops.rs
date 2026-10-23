//! What a remote-desktop tab asks of a host: connecting (through the host's
//! SSH tunnel when it has one), attaching the sink frames go to, input,
//! frame acknowledgements, pausing, and disconnecting.
//!
//! These were the bodies of the Tauri commands of the same names; error text
//! is unchanged, because the Vue frontend shows it as is.
//!
//! The rules in `vnc/` hold for every call here. Frames go to the
//! [`ByteSink`] given to [`Core::vnc_open_frame_channel`], and the session
//! requests none until one is attached (which also forces a full update, so
//! the shadow is invalidated for the new canvas). The renderer acknowledges
//! each frame it has applied with [`Core::vnc_frame_ack`]; that is the flow
//! control. A hidden tab says so with [`Core::vnc_set_paused`] rather than
//! withholding acknowledgements. The tunnel is its own SSH session
//! (`vnc_tunnel_port`) and is dropped by `release_vnc_host` once no session
//! on the host uses it.
//!
//! The password helpers (`store_vnc_secret`, `has_vnc_secret`,
//! `clear_vnc_secret`) are in `hosts.rs`, beside the other kinds', and so is
//! `vnc_disconnect_host`.

use crate::events::{ByteSink, Events};
use crate::state::{load_vnc_password, release_vnc_host, vnc_dial_target, with_db, with_vnc};
use crate::{db, uri, vnc, Core};

impl Core {
    /// Connect and authenticate, then start the session thread.
    ///
    /// `password` overrides the stored VNC password; `ssh_password` the
    /// tunnel's SSH password, for the prompt-and-retry path. Session events
    /// (`VncBell`, `VncClipboard`, `VncResize`, `VncDisconnected`,
    /// `VncError`) go to `events`, keyed by the returned session id.
    pub async fn vnc_connect(
        &self,
        events: Events,
        host_id: i64,
        password: Option<String>,
        ssh_password: Option<String>,
    ) -> Result<vnc::VncSessionInfo, String> {
        let host = with_db(self, |conn| db::get_host_by_id(conn, host_id))?
            .ok_or_else(|| format!("host {} not found", host_id))?;

        let (address, port, tunnelled) = vnc_dial_target(self, &host, ssh_password).await?;

        let vnc_password = match password {
            Some(p) => Some(p),
            None => load_vnc_password(host_id)?,
        };
        let session_id = format!("vnc-{}", uuid::Uuid::new_v4());
        let view_only = host.vnc_view_only != 0;
        let username = host.vnc_username.clone();

        let session_id_for_thread = session_id.clone();
        let (handle, info) = tokio::task::spawn_blocking(move || {
            vnc::start(
                events,
                session_id_for_thread,
                host_id,
                &address,
                port,
                vnc_password.as_deref(),
                username.as_deref(),
                tunnelled,
                view_only,
            )
        })
        .await
        .map_err(|e| format!("VNC connect task panicked: {}", e))?
        .map_err(|e| uri::redact_uris_in_text(&e))?;

        self.vnc_sessions
            .lock()
            .map_err(|_| "session registry poisoned".to_string())?
            .insert(session_id, handle);

        Ok(info)
    }

    /// Hand the session the sink its frames go down.
    ///
    /// Until this arrives the thread requests no frames at all, which is what
    /// removes the dropped-first-frame race rather than papering over it.
    pub fn vnc_open_frame_channel(
        &self,
        session_id: &str,
        sink: Box<dyn ByteSink>,
    ) -> Result<(), String> {
        with_vnc(self, session_id, |handle| {
            handle.attach_channel(sink);
            Ok(())
        })
    }

    /// Close one session, and the host's tunnel when it was the last one.
    pub async fn vnc_disconnect(&self, session_id: &str) -> Result<(), String> {
        let host_id = {
            let mut sessions = self
                .vnc_sessions
                .lock()
                .map_err(|_| "session registry poisoned".to_string())?;
            sessions.remove(session_id).map(|h| {
                h.disconnect();
                h.host_id
            })
        };
        if let Some(host_id) = host_id {
            release_vnc_host(self, host_id).await;
        }
        Ok(())
    }

    pub fn vnc_pointer(
        &self,
        session_id: &str,
        x: u16,
        y: u16,
        button_mask: u8,
    ) -> Result<(), String> {
        with_vnc(self, session_id, |handle| {
            handle.send_input(vnc::proto::encode_pointer_event(button_mask, x, y));
            Ok(())
        })
    }

    pub fn vnc_key(&self, session_id: &str, keysym: u32, down: bool) -> Result<(), String> {
        with_vnc(self, session_id, |handle| {
            handle.send_input(vnc::proto::encode_key_event(down, keysym));
            Ok(())
        })
    }

    /// Press every keysym in order, then release them in reverse.
    ///
    /// Exists for Ctrl+Alt+Delete, which no browser will ever deliver as a
    /// keydown -- Windows intercepts it and Linux window managers grab it.
    pub fn vnc_key_sequence(&self, session_id: &str, keysyms: &[u32]) -> Result<(), String> {
        with_vnc(self, session_id, |handle| {
            let mut bytes = Vec::new();
            for &keysym in keysyms {
                bytes.extend_from_slice(&vnc::proto::encode_key_event(true, keysym));
            }
            for &keysym in keysyms.iter().rev() {
                bytes.extend_from_slice(&vnc::proto::encode_key_event(false, keysym));
            }
            handle.send_input(bytes);
            Ok(())
        })
    }

    /// The renderer has applied frame `seq`; the next may be requested.
    pub fn vnc_frame_ack(&self, session_id: &str, seq: u32) -> Result<(), String> {
        with_vnc(self, session_id, |handle| {
            handle.ack(seq);
            Ok(())
        })
    }

    /// Make the next request a full update (which also invalidates the
    /// shadow, so every pixel is sent again).
    pub fn vnc_refresh(&self, session_id: &str) -> Result<(), String> {
        with_vnc(self, session_id, |handle| {
            handle.refresh();
            Ok(())
        })
    }

    /// Stop or resume requesting frames while the panel is out of sight.
    pub fn vnc_set_paused(&self, session_id: &str, paused: bool) -> Result<(), String> {
        with_vnc(self, session_id, |handle| {
            handle.set_paused(paused);
            Ok(())
        })
    }

    pub fn vnc_set_view_only(&self, session_id: &str, view_only: bool) -> Result<(), String> {
        with_vnc(self, session_id, |handle| {
            handle.set_view_only(view_only);
            Ok(())
        })
    }

    /// Push the local clipboard to the remote.
    ///
    /// Returns whether anything was lost: the base protocol's cut text is
    /// ISO-8859-1 and has nowhere to put a character outside it, so the caller
    /// can say so once rather than silently replacing it.
    pub fn vnc_send_clipboard(&self, session_id: &str, text: &str) -> Result<bool, String> {
        with_vnc(self, session_id, |handle| {
            if handle.is_view_only() {
                return Ok(false);
            }
            let (bytes, lossy) = vnc::proto::encode_client_cut_text(text);
            handle.send_input(bytes);
            Ok(lossy)
        })
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use crate::events::NoEvents;
    use crate::state::tests_support::memory_core;

    #[test]
    fn calls_on_a_closed_session_say_so() {
        // The Vue panel ignores these errors, but the text is the contract.
        let core = memory_core();
        let gone = "this VNC session is no longer open";
        assert_eq!(core.vnc_pointer("vnc-x", 1, 2, 0).unwrap_err(), gone);
        assert_eq!(core.vnc_key("vnc-x", 0x61, true).unwrap_err(), gone);
        assert_eq!(core.vnc_key_sequence("vnc-x", &[1]).unwrap_err(), gone);
        assert_eq!(core.vnc_frame_ack("vnc-x", 3).unwrap_err(), gone);
        assert_eq!(core.vnc_refresh("vnc-x").unwrap_err(), gone);
        assert_eq!(core.vnc_set_paused("vnc-x", true).unwrap_err(), gone);
        assert_eq!(core.vnc_set_view_only("vnc-x", true).unwrap_err(), gone);
        assert_eq!(core.vnc_send_clipboard("vnc-x", "t").unwrap_err(), gone);
    }

    #[test]
    fn a_missing_host_is_named_and_an_unknown_session_closes_quietly() {
        let core = memory_core();
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let err = core
                .vnc_connect(Arc::new(NoEvents), 7, None, None)
                .await
                .unwrap_err();
            assert_eq!(err, "host 7 not found");
            assert_eq!(core.vnc_disconnect("vnc-x").await, Ok(()));
        });
    }
}
