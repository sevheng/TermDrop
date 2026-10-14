//! VNC (RFB) client.
//!
//! The protocol is implemented here rather than taken from a crate for two
//! reasons. The maintained option, `vnc-rs`, is async and wants `tokio`
//! features this app does not enable, and it does not speak Apple's
//! authentication — which would have to be hand-written regardless, and which
//! is entangled with the handshake it *does* own. The older `vnc` crate has no
//! Tight encoding, which is what every real server negotiates.
//!
//! The layers are deliberately separate so that almost all of the protocol is
//! testable without a socket:
//!
//! - [`proto`] frames messages. Big-endian, pure, no I/O.
//! - [`auth`] answers challenges. Pure, no I/O.
//! - [`pixel`] describes pixel layouts and converts them to RGBA.
//! - [`frame`] builds the little-endian frames the canvas renderer consumes.
//!
//! On top of those sit [`conn`] (the socket and the handshake), [`encoding`]
//! (rectangle decoders) and [`session`] (the thread that drives one
//! connection).

pub mod auth;
pub mod conn;
pub mod encoding;
pub mod frame;
pub mod pixel;
pub mod proto;
pub mod session;

pub use session::{start, VncSessionHandle, VncSessionInfo};
