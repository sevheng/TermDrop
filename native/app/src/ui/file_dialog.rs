//! Native open, save and folder dialogs, over `rfd`. The Tauri app used its
//! dialog plugin for the same jobs: choosing a key file, an export's
//! destination and an import's source, and the datastore tabs' backup files
//! and dump folders.
//!
//! The dialog runs on rfd's own GTK thread (on Linux) and is awaited here,
//! so the window keeps painting while it is open. The answer comes back on
//! the UI thread with the window, so the caller can toast or open a dialog.
//! Cancelling calls nothing.

use std::path::PathBuf;

use gpui_kit::{App, Window};

/// A file filter: a name and its extensions (`("JSON", &["json"])`).
pub type Filter<'a> = (&'a str, &'a [&'a str]);

/// Ask for an existing file.
pub fn pick_file(
    window: &Window,
    cx: &App,
    title: &str,
    filter: Option<Filter<'_>>,
    on_pick: impl FnOnce(PathBuf, &mut Window, &mut App) + 'static,
) {
    let mut dialog = rfd::AsyncFileDialog::new().set_title(title);
    if let Some((name, exts)) = filter {
        dialog = dialog.add_filter(name, exts);
    }
    if let Some(home) = std::env::var_os("HOME") {
        dialog = dialog.set_directory(home);
    }
    let chosen = dialog.pick_file();
    window
        .spawn(cx, async move |cx| {
            if let Some(file) = chosen.await {
                let path = file.path().to_path_buf();
                let _ = cx.update(|window, cx| on_pick(path, window, cx));
            }
        })
        .detach();
}

/// Ask where to write a new file, suggesting `file_name`.
pub fn save_file(
    window: &Window,
    cx: &App,
    title: &str,
    file_name: &str,
    filter: Option<Filter<'_>>,
    on_pick: impl FnOnce(PathBuf, &mut Window, &mut App) + 'static,
) {
    let mut dialog = rfd::AsyncFileDialog::new()
        .set_title(title)
        .set_file_name(file_name)
        .set_can_create_directories(true);
    if let Some((name, exts)) = filter {
        dialog = dialog.add_filter(name, exts);
    }
    if let Some(home) = std::env::var_os("HOME") {
        dialog = dialog.set_directory(home);
    }
    let chosen = dialog.save_file();
    window
        .spawn(cx, async move |cx| {
            if let Some(file) = chosen.await {
                let path = file.path().to_path_buf();
                let _ = cx.update(|window, cx| on_pick(path, window, cx));
            }
        })
        .detach();
}

/// Ask for an existing folder (a MongoDB dump's destination or source).
pub fn pick_folder(
    window: &Window,
    cx: &App,
    title: &str,
    on_pick: impl FnOnce(PathBuf, &mut Window, &mut App) + 'static,
) {
    let mut dialog = rfd::AsyncFileDialog::new()
        .set_title(title)
        .set_can_create_directories(true);
    if let Some(home) = std::env::var_os("HOME") {
        dialog = dialog.set_directory(home);
    }
    let chosen = dialog.pick_folder();
    window
        .spawn(cx, async move |cx| {
            if let Some(folder) = chosen.await {
                let path = folder.path().to_path_buf();
                let _ = cx.update(|window, cx| on_pick(path, window, cx));
            }
        })
        .detach();
}
