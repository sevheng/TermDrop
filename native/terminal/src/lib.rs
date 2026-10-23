//! The terminal element: a GPUI view over `alacritty_terminal`.
//!
//! It knows nothing about SSH or PTYs. Output goes in through a
//! [`TerminalFeed`] from any thread; keystrokes, pastes, mouse reports and the
//! terminal's own replies come out through the `input` callback; the grid size
//! comes out through `on_resize`. SSH tabs, local shells and the Docker exec
//! pane all plug into those three (interface 3 in docs/gpui-migration-plan.md).
//!
//! ```ignore
//! let view = TerminalView::new(window, cx, TerminalOptions::default(),
//!     Box::new(move |bytes| writer.send(bytes)),
//!     Box::new(move |cols, rows| pty.resize(cols, rows)));
//! let feed = view.read(cx).feed();
//! std::thread::spawn(move || while let Ok(n) = reader.read(&mut buf) { feed.feed(&buf[..n]) });
//! ```

mod element;
mod feed;
pub mod keys;
pub mod mouse;
mod palette;
pub mod paste;
mod search;
pub mod url;
mod view;

pub use feed::{FrameStats, TerminalFeed};
pub use palette::Palette;
pub use search::SearchDirection;
pub use view::{
    init, TerminalEvent, TerminalOptions, TerminalView, DEFAULT_FONT_FAMILY, KEY_CONTEXT,
    NERD_SYMBOLS_FAMILY,
};

#[cfg(test)]
mod selection_tests;
