//! TermDrop's backend, shared by the Tauri app (`src-tauri`) and the native
//! GPUI app.
//!
//! Nothing here depends on a UI toolkit. What the backend reports unasked goes
//! through [`EventSink`] as a typed [`CoreEvent`], and terminal output and VNC
//! frames go through a [`ByteSink`]; each app supplies its own. Module paths
//! are the ones these files had under `src-tauri/src`, so `db::get_hosts`,
//! `ssh::trust::TRUST_PREFIX` and the rest keep their names.

// Operations mirror the Tauri command payloads one argument per field, as they
// did when this code lived in the Tauri crate under the same allowance;
// grouping them into structs would change that IPC contract.
#![allow(clippy::too_many_arguments)]

pub mod crypto;
pub mod db;
pub mod dev_perf;
pub mod docker;
pub mod error;
pub mod events;
pub mod exec_ops;
pub mod forward_ops;
pub mod hosts;
pub mod local_pty;
pub mod local_shells;
pub mod mongo_ops;
pub mod mongodb;
pub mod port_forward;
pub mod redis;
pub mod redis_backup;
pub mod redis_ops;
pub mod security;
pub mod sftp;
pub mod sftp_ops;
pub mod ssh;
pub mod ssh_config_parser;
pub mod state;
pub mod system;
pub mod uri;
pub mod vnc;
pub mod vnc_ops;

pub use error::{CoreError, CoreResult, TrustPrompt};
pub use events::{ByteSink, CoreEvent, EventSink, Events, PtyKind};
pub use state::{Cached, Core, MongoOpHandle};
