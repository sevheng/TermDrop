//! Reaching a host through another one (OpenSSH's `ProxyJump`).
//!
//! **Why a loopback socket and not the channel itself.** libssh2 reads and
//! writes the session's file descriptor directly — `Session::set_tcp_stream`
//! requires `AsRawFd` / `AsRawSocket` — so an `ssh2::Channel` to the target
//! cannot be handed to a second `Session`. Instead the bastion's
//! `direct-tcpip` channel is pumped into one half of a connected loopback pair,
//! and the target's session runs over the other half as if it were an ordinary
//! TCP connection.
//!
//! **It cleans up after itself.** The pump thread owns the bastion session.
//! When the target session is dropped its socket closes, the pump reads EOF
//! and returns, and the bastion session drops with it. If the bastion dies the
//! reverse happens. There is no registry and no refcount to get wrong.
//!
//! Each call opens its own bastion session, so every target session has one
//! underneath it. That keeps "never two commands on one `ssh2::Session`" true
//! without having to reason about it.

use super::session::{self, Accept, SshTarget};
use crate::ssh::trust;
use std::net::{Shutdown, TcpListener, TcpStream};

/// Connect to `target_host:target_port` through `jump`, and return the local
/// end of the stream.
///
/// `target_host` is resolved **by the bastion**, which is the point: a name like
/// `db.internal` only means something on the far side.
///
/// A trust refusal from the bastion comes back marked `hop: "jump"`, so the
/// frontend knows which acceptance to send on the retry.
pub fn open_via(
    jump: &SshTarget,
    target_host: &str,
    target_port: u16,
    accept_jump: Option<&str>,
) -> Result<TcpStream, String> {
    let bastion = session::create_exec_session(
        jump,
        &Accept {
            target: accept_jump.map(String::from),
            jump: None,
        },
    )
    .map_err(|e| {
        let e = trust::mark_jump_hop(e);
        if e.starts_with(trust::TRUST_PREFIX) {
            e
        } else {
            format!("jump host {}: {}", jump.host, e)
        }
    })?;

    // Opened before the loopback pair exists, so the usual reasons this fails —
    // forwarding disabled on the bastion, or a target it cannot reach — are
    // reported as themselves rather than as a handshake that never answers.
    let mut channel = bastion
        .channel_direct_tcpip(target_host, target_port, None)
        .map_err(|e| {
            format!(
                "jump host {} could not reach {}:{}: {}. The SSH server may have \
                 AllowTcpForwarding disabled, or it may not be able to reach that address.",
                jump.host, target_host, target_port, e
            )
        })?;

    let (local, mut remote) = loopback_pair()?;

    bastion.set_blocking(false);
    remote
        .set_nonblocking(true)
        .map_err(|e| format!("jump socket: {}", e))?;

    let label = format!("{}:{} via {}", target_host, target_port, jump.host);
    std::thread::spawn(move || {
        // Owned here so it lives exactly as long as the pump does.
        let _bastion = bastion;
        if let Err(e) = crate::port_forward::pipe_bidirectional_nb(&mut remote, &mut channel) {
            tracing::warn!("jump {}: {}", label, e);
        }
        let _ = channel.close();
        let _ = remote.shutdown(Shutdown::Both);
        tracing::debug!("jump {} closed", label);
    });

    Ok(local)
}

/// A connected pair of loopback sockets.
///
/// The listener is bound to `127.0.0.1:0` and accepts exactly once. Any
/// process on this machine could connect to that port in the window between
/// bind and accept, so the accepted peer must be *our* client — otherwise a
/// local process would be handed the plaintext of the target's SSH stream
/// (still encrypted end to end, but ours to protect).
fn loopback_pair() -> Result<(TcpStream, TcpStream), String> {
    let listener =
        TcpListener::bind("127.0.0.1:0").map_err(|e| format!("bind a jump socket: {}", e))?;
    let addr = listener
        .local_addr()
        .map_err(|e| format!("jump socket address: {}", e))?;

    let client = TcpStream::connect(addr).map_err(|e| format!("jump socket connect: {}", e))?;
    let expected = client
        .local_addr()
        .map_err(|e| format!("jump socket address: {}", e))?;

    let (server, peer) = listener
        .accept()
        .map_err(|e| format!("jump socket accept: {}", e))?;
    if peer != expected {
        return Err(format!(
            "jump socket: an unexpected local connection from {} arrived first",
            peer
        ));
    }
    Ok((client, server))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};

    #[test]
    fn the_loopback_pair_is_connected_both_ways() {
        let (mut a, mut b) = loopback_pair().unwrap();
        a.write_all(b"ping").unwrap();
        let mut buf = [0u8; 4];
        b.read_exact(&mut buf).unwrap();
        assert_eq!(&buf, b"ping");

        b.write_all(b"pong").unwrap();
        a.read_exact(&mut buf).unwrap();
        assert_eq!(&buf, b"pong");
    }

    #[test]
    fn the_pair_is_bound_to_loopback_only() {
        let (a, b) = loopback_pair().unwrap();
        assert!(a.peer_addr().unwrap().ip().is_loopback());
        assert!(b.local_addr().unwrap().ip().is_loopback());
    }

    #[test]
    fn an_unreachable_bastion_names_itself() {
        // A listener that hangs up at once: the handshake fails immediately.
        // (A closed port is not enough -- some sandboxes drop the SYN rather
        // than refuse it, and the test then waits out the connect timeout.)
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            if let Ok((stream, _)) = listener.accept() {
                drop(stream);
            }
        });
        let jump = SshTarget {
            host: "127.0.0.1".into(),
            port,
            username: "u".into(),
            password: Some("p".into()),
            ..Default::default()
        };
        let err = open_via(&jump, "db.internal", 22, None).unwrap_err();
        assert!(err.starts_with("jump host 127.0.0.1:"), "{}", err);
    }
}

/// Two real hops, against the Redis compose's bastion.
///
/// The bastion is both hops: it is reached on `127.0.0.1:2222`, and from inside
/// it `localhost:2222` is its own sshd again. That is a genuine second SSH
/// session carried over a `direct-tcpip` channel, which is all a jump is, and
/// it needs no extra container.
///
/// Also drives the whole trust flow: each hop is refused as unknown, the
/// refusal names its hop, and the retry with that hop's fingerprint gets
/// through. Keys are recorded in a scratch store, never the real one.
///
/// ```text
/// docker compose -f docker-compose.redis.yml up -d --no-deps bastion   # from the top level
/// cargo test ssh::jump::live -- --ignored --test-threads=1            # from src-tauri/
/// ```
#[cfg(test)]
mod live {
    use super::*;
    use crate::ssh::exec::run_command;

    fn bastion() -> SshTarget {
        SshTarget {
            host: "127.0.0.1".into(),
            port: 2222,
            username: "tunnel".into(),
            password: Some("tunnelpass".into()),
            ..Default::default()
        }
    }

    fn through_bastion(port: u16) -> SshTarget {
        SshTarget {
            host: "localhost".into(),
            port,
            jump: Some(Box::new(bastion())),
            ..bastion()
        }
    }

    fn refusal(err: &str) -> serde_json::Value {
        let json = err
            .strip_prefix(trust::TRUST_PREFIX)
            .unwrap_or_else(|| panic!("expected a trust refusal, got: {}", err));
        serde_json::from_str(json).unwrap()
    }

    /// A fresh, empty trust store. Tests run with `--test-threads=1`, which is
    /// what makes setting a process-wide variable safe here.
    fn scratch_store() -> std::path::PathBuf {
        let path = std::env::temp_dir().join(format!("termdrop-jump-{}", std::process::id()));
        let _ = std::fs::remove_file(&path);
        std::env::set_var("TERMDROP_KNOWN_HOSTS", &path);
        path
    }

    #[test]
    #[ignore]
    fn each_hop_is_asked_about_once_and_then_a_command_runs() {
        let store = scratch_store();
        let target = through_bastion(2222);

        // The user's own known_hosts may already trust the bastion, so the
        // first refusal is either hop -- but whichever it is must say so.
        // Up to two refusals, then the attempt that carries both acceptances.
        let mut accept = Accept::none();
        let mut connected = false;
        for _ in 0..3 {
            match session::create_exec_session(&target, &accept) {
                Ok(_) => {
                    connected = true;
                    break;
                }
                Err(e) => {
                    let r = refusal(&e);
                    assert_eq!(r["kind"], "unknown", "{}", e);
                    let fp = r["fingerprint"].as_str().unwrap().to_string();
                    match r["hop"].as_str().unwrap() {
                        "jump" => {
                            assert_eq!(r["host"], "127.0.0.1");
                            accept.jump = Some(fp);
                        }
                        "target" => {
                            // The target's real name, never the loopback socket.
                            assert_eq!(r["host"], "localhost");
                            accept.target = Some(fp);
                        }
                        other => panic!("unexpected hop {}", other),
                    }
                }
            }
        }
        assert!(connected, "still refused after accepting both hops");

        // Accepted keys are now on file: no acceptance is needed any more.
        let session = session::create_exec_session(&target, &Accept::none())
            .expect("both hops trusted: bastion must be up (see module docs)");
        let out = run_command(&session, "echo jumped").unwrap();
        assert_eq!(out.trim(), "jumped");

        let _ = std::fs::remove_file(store);
    }

    #[test]
    #[ignore]
    fn an_unreachable_target_is_reported_against_the_jump_host() {
        let _store = scratch_store();
        // Nothing listens on 1 inside the container. The bastion's key may be
        // unknown in the scratch store, so accept it first.
        let target = through_bastion(1);
        let err = match session::create_exec_session(&target, &Accept::none()) {
            Err(e) if e.starts_with(trust::TRUST_PREFIX) => {
                let fp = refusal(&e)["fingerprint"].as_str().unwrap().to_string();
                let accept = Accept {
                    target: None,
                    jump: Some(fp),
                };
                match session::create_exec_session(&target, &accept) {
                    Err(e) => e,
                    Ok(_) => panic!("nothing listens on port 1"),
                }
            }
            Err(e) => e,
            Ok(_) => panic!("nothing listens on port 1"),
        };
        assert!(err.contains("could not reach localhost:1"), "{}", err);
    }
}
