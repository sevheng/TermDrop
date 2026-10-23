//! TermDrop's native (GPUI Kit) shell. See docs/gpui-migration-plan.md,
//! Phase 2 and "Parallel execution", interface 2.
//!
//! - [`app`]: `AppModel` and the pure rules it is built from.
//! - [`ui`]: the theme and the building blocks every panel uses.
//! - [`shell`]: the window, sidebar, tab strip and settings.
//! - [`backend`]: the seams the core and terminal tracks plug into.
//! - [`core_backend`] and [`tabs`]: those seams filled from `termdrop-core`
//!   and `termdrop-terminal`.
//! - [`panels`]: the right-hand panel beside an SSH tab (SFTP, Tunnels,
//!   Docker, Security); its `README.md` is the contract for each panel.
//! - [`datastores`]: the MongoDB, Redis and remote-desktop (VNC) tabs; its
//!   `README.md` is the contract for each.

// The theme config is one `serde_json::json!` object with a key per GPUI Kit
// colour slot, which is deeper than the macro's default recursion budget.
#![recursion_limit = "512"]

pub mod app;
pub mod backend;
pub mod core_backend;
pub mod datastores;
pub mod panels;
pub mod shell;
pub mod single_instance;
pub mod tabs;
pub mod ui;

use std::rc::Rc;
use std::sync::Arc;

use gpui_kit::component::TitleBar;
use gpui_kit::{AppContext as _, WindowOptions, px, size};

use app::AppModel;
use backend::{HostStore, TabContentFactory};
use shell::main_window::{MainWindow, bind_keys};
use shell::window_state::{self, MIN_SIZE, WINDOW_STATE_KEY, WindowState};

/// The name the preview installs and shows under. It installs beside the
/// Tauri app ("TermDrop", `com.termdrop`) rather than over it; see
/// `native/RELEASING.md`. The packaging metadata in `app/Cargo.toml` carries
/// the same two values.
pub const PRODUCT_NAME: &str = "TermDrop Native Preview";
/// The preview's identifier: bundle id, single-instance key.
pub const IDENTIFIER: &str = "com.termdrop.native-preview";
/// The Linux desktop file's name (cargo-packager names it after the
/// binary), so a Wayland compositor can match the window to its icon.
const APP_ID: &str = "termdrop-native";

/// Open the app over `store`, with `factory` filling each tab: the core
/// adapter and terminals normally, the fake backend and placeholders with
/// `--demo`. Each message on `focus_requests` (a second launch, see
/// [`single_instance`]) brings the window forward.
pub fn run(
    store: Arc<dyn HostStore>,
    factory: Rc<dyn TabContentFactory>,
    focus_requests: Option<async_channel::Receiver<()>>,
) {
    gpui_kit::application()
        .with_assets(gpui_kit::assets::AllAssets)
        .run(move |cx| {
            // First: the theme and every terminal resolve their families
            // against the fonts registered at that moment.
            ui::fonts::register(cx);
            gpui_kit::init(cx);
            // After the kit: its root binds Tab and Ctrl+C, and the terminal
            // rebinds them in its own context so they reach the program.
            termdrop_terminal::init(cx);
            bind_keys(cx);
            // Last: it copies every chord bound so far into the remote
            // screen's context, so they reach the remote instead.
            datastores::vnc::init(cx);
            let model = cx.new(|cx| AppModel::new(store, cx));
            factory.start(&model, cx);
            // Before the window opens, so the first frame is in the saved theme.
            ui::theme::install(model.read(cx).theme(), cx);

            let saved = model
                .read(cx)
                .setting(WINDOW_STATE_KEY)
                .and_then(|raw| WindowState::decode(&raw));
            let (bounds, display_id) = window_state::window_bounds(saved.as_ref(), cx);
            let options = WindowOptions {
                window_bounds: Some(bounds),
                display_id,
                window_min_size: Some(size(px(MIN_SIZE.0), px(MIN_SIZE.1))),
                app_id: Some(APP_ID.into()),
                ..TitleBar::window_options()
            };
            gpui_kit::open_window(options, cx, |window, cx| {
                cx.new(|cx| MainWindow::new(model, factory, window, cx))
            })
            .expect("failed to open the main window");

            if let Some(requests) = focus_requests {
                cx.spawn(async move |cx| {
                    while requests.recv().await.is_ok() {
                        cx.update(|cx| {
                            cx.activate(true);
                            for handle in cx.windows() {
                                let _ = handle.update(cx, |_, window, _| window.activate_window());
                            }
                        });
                    }
                })
                .detach();
            }

            // Closing the last window ends the app, on every platform.
            cx.on_window_closed(|cx, _| {
                if cx.windows().is_empty() {
                    cx.quit();
                }
            })
            .detach();
        });
}
