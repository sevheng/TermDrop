//! Native open, save and folder dialogs, over `rfd`. The Tauri app used its
//! dialog plugin for the same jobs: choosing a key file, an export's
//! destination and an import's source, and the datastore tabs' backup files
//! and dump folders.
//!
//! The dialog runs on rfd's own GTK thread (on Linux) and is awaited here,
//! so the window keeps painting while it is open. The answer comes back on
//! the UI thread with the window, so the caller can toast or open a dialog.
//! Cancelling calls nothing.
//!
//! On Linux the dialog is GTK, which picks its display server on its own:
//! with `WAYLAND_DISPLAY` unset it still tries the default `wayland-0`
//! socket, while GPUI then runs on X11. Under WSLg (and any session where
//! `WAYLAND_DISPLAY` was unset to force X11) the dialog opened as a Wayland
//! window that no X11 tool could see, and looked like a click that did
//! nothing. [`align_gtk_backend`] makes GTK follow GPUI's choice.

use std::path::PathBuf;

use gpui_kit::{App, Window};

/// A file filter: a name and its extensions (`("JSON", &["json"])`).
pub type Filter<'a> = (&'a str, &'a [&'a str]);

/// The Vue dialogs' `{ name: 'All files', extensions: ['*'] }`, last in a
/// list so the others stay the default. rfd turns each extension into a
/// `*.ext` glob, so this is `*.*`: a file with no extension at all is not
/// listed. The Tauri dialog plugin goes through rfd too and does the same.
pub const ALL_FILES: Filter<'static> = ("All files", &["*"]);

/// Add `filters` in order; the first is the one the dialog starts on.
fn with_filters(mut dialog: rfd::AsyncFileDialog, filters: &[Filter<'_>]) -> rfd::AsyncFileDialog {
    for (name, exts) in filters {
        dialog = dialog.add_filter(*name, exts);
    }
    dialog
}

/// The `GDK_BACKEND` GTK should be given so its dialogs open on the display
/// server GPUI chose, or `None` to leave GTK alone. GPUI uses Wayland only
/// when `WAYLAND_DISPLAY` is set and X11 otherwise; an explicit
/// `GDK_BACKEND` is the user's and is kept.
pub fn gdk_backend_for(
    wayland_display: Option<&str>,
    display: Option<&str>,
    gdk_backend: Option<&str>,
) -> Option<&'static str> {
    let set = |v: Option<&str>| v.is_some_and(|v| !v.is_empty());
    if set(gdk_backend) || set(wayland_display) || !set(display) {
        return None;
    }
    Some("x11")
}

/// Point GTK at GPUI's display server (see the module docs). Call first
/// thing in `main`, while the process has one thread: setting an
/// environment variable is only sound then.
pub fn align_gtk_backend() {
    if !cfg!(target_os = "linux") {
        return;
    }
    let var = |k: &str| std::env::var(k).ok();
    if let Some(backend) = gdk_backend_for(
        var("WAYLAND_DISPLAY").as_deref(),
        var("DISPLAY").as_deref(),
        var("GDK_BACKEND").as_deref(),
    ) {
        // SAFETY: called from `main` before any thread is spawned, so no
        // other thread can be reading the environment.
        unsafe { std::env::set_var("GDK_BACKEND", backend) };
    }
}

/// Ask for an existing file.
pub fn pick_file(
    window: &Window,
    cx: &App,
    title: &str,
    filters: &[Filter<'_>],
    on_pick: impl FnOnce(PathBuf, &mut Window, &mut App) + 'static,
) {
    let mut dialog = with_filters(rfd::AsyncFileDialog::new().set_title(title), filters);
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
    filters: &[Filter<'_>],
    on_pick: impl FnOnce(PathBuf, &mut Window, &mut App) + 'static,
) {
    let mut dialog = with_filters(
        rfd::AsyncFileDialog::new()
            .set_title(title)
            .set_file_name(file_name)
            .set_can_create_directories(true),
        filters,
    );
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gtk_follows_gpui_onto_x11_only_when_gpui_is_there() {
        // WAYLAND_DISPLAY unset, DISPLAY set: GPUI is on X11, GTK must be.
        assert_eq!(gdk_backend_for(None, Some(":0"), None), Some("x11"));
        assert_eq!(gdk_backend_for(Some(""), Some(":0"), None), Some("x11"));
        // GPUI is on Wayland: GTK's own choice already matches.
        assert_eq!(gdk_backend_for(Some("wayland-0"), Some(":0"), None), None);
        // No X display to point at.
        assert_eq!(gdk_backend_for(None, None, None), None);
        // The user's explicit choice wins.
        assert_eq!(gdk_backend_for(None, Some(":0"), Some("wayland")), None);
    }
}
