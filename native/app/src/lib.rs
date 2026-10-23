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
//! - [`datastores`]: the MongoDB and Redis tabs; its `README.md` is the
//!   contract for each.

// The theme config is one `serde_json::json!` object with a key per GPUI Kit
// colour slot, which is deeper than the macro's default recursion budget.
#![recursion_limit = "512"]

pub mod app;
pub mod backend;
pub mod core_backend;
pub mod datastores;
pub mod panels;
pub mod shell;
pub mod tabs;
pub mod ui;

use std::rc::Rc;
use std::sync::Arc;

use gpui_kit::component::TitleBar;
use gpui_kit::{AppContext as _, Bounds, WindowBounds, WindowOptions, px, size};

use app::AppModel;
use backend::{HostStore, TabContentFactory};
use shell::main_window::{MainWindow, bind_keys};

/// Open the app over `store`, with `factory` filling each tab: the core
/// adapter and terminals normally, the fake backend and placeholders with
/// `--demo`.
pub fn run(store: Arc<dyn HostStore>, factory: Rc<dyn TabContentFactory>) {
    gpui_kit::application()
        .with_assets(gpui_kit::assets::AllAssets)
        .run(move |cx| {
            gpui_kit::init(cx);
            // After the kit: its root binds Tab and Ctrl+C, and the terminal
            // rebinds them in its own context so they reach the program.
            termdrop_terminal::init(cx);
            bind_keys(cx);
            let model = cx.new(|cx| AppModel::new(store, cx));
            factory.start(&model, cx);
            // Before the window opens, so the first frame is in the saved theme.
            ui::theme::install(model.read(cx).theme(), cx);

            let options = WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                    None,
                    size(px(1200.), px(760.)),
                    cx,
                ))),
                window_min_size: Some(size(px(720.), px(420.))),
                ..TitleBar::window_options()
            };
            gpui_kit::open_window(options, cx, |window, cx| {
                cx.new(|cx| MainWindow::new(model, factory, window, cx))
            })
            .expect("failed to open the main window");

            // Closing the last window ends the app, on every platform.
            cx.on_window_closed(|cx, _| {
                if cx.windows().is_empty() {
                    cx.quit();
                }
            })
            .detach();
        });
}
