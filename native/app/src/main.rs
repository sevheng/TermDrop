use std::rc::Rc;
use std::sync::Arc;

use termdrop_gpui::backend::{FakeBackend, PlaceholderContent};
use termdrop_gpui::core_backend::{self, CoreStore};
use termdrop_gpui::tabs::CoreContent;

fn main() {
    init_logging();
    // `--demo` runs on fictional hosts, in-memory settings and placeholder
    // tabs: nothing reads or writes the real database, keyring or network.
    if std::env::args().skip(1).any(|a| a == "--demo") {
        termdrop_gpui::run(Arc::new(FakeBackend::new()), Rc::new(PlaceholderContent));
        return;
    }
    // The Tauri app's database, so both apps see the same hosts, settings
    // and trusted host keys.
    let (services, events) = match core_backend::open() {
        Ok(opened) => opened,
        Err(e) => {
            eprintln!("TermDrop could not open its database: {e}");
            std::process::exit(1);
        }
    };
    let store = Arc::new(CoreStore::new(services.core.clone()));
    termdrop_gpui::run(store, Rc::new(CoreContent::new(services, events)));
}

/// Logs to stderr, filtered by `RUST_LOG` (default `info`), as the Tauri app's
/// console layer is. Not to its rolling file: two processes appending to one
/// daily log is a packaging decision, not one to make here.
fn init_logging() {
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));
    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .with_ansi(false)
        .try_init();
}
