//! The sidebar's actions that ask first or touch a file: deleting a host or
//! a group, and importing or exporting hosts as JSON. Ports of `deleteHost`,
//! `deleteGroup` and `onImportFileSelected` in `HostSidebar.vue` and
//! `exportHosts` in `stores/connection.js`, with their wording.

use gpui_kit::{App, Entity, Window};

use crate::app::AppModel;
use crate::app::host_import::{
    MAX_IMPORT_BYTES, export_file_name, normalize_import_host, parse_hosts_file, summarize_import,
};
use crate::app::host_menu::{delete_group_message, delete_host_message};
use crate::backend::ImportEntry;
use crate::shell::ssh_import::summary_toast;
use crate::ui::dialogs::{ConfirmOptions, confirm};
use crate::ui::file_dialog;
use crate::ui::toast::{ToastKind, toast};

const JSON: file_dialog::Filter<'static> = ("JSON", &["json"]);

/// Ask, then delete the host and close every tab it has open.
pub fn confirm_delete_host(
    window: &mut Window,
    cx: &mut App,
    model: Entity<AppModel>,
    host_id: i64,
) {
    let Some(host) = model.read(cx).host(host_id).cloned() else {
        return;
    };
    confirm(
        window,
        cx,
        ConfirmOptions::new("Delete Host", delete_host_message(&host))
            .confirm_label("Delete")
            .danger(),
        move |yes, window, cx| {
            if !yes {
                return;
            }
            if let Err(e) = model.update(cx, |m, cx| m.delete_host(host_id, cx)) {
                toast(
                    window,
                    cx,
                    format!("Could not delete the host: {e}"),
                    ToastKind::Error,
                );
            }
        },
    );
}

/// Ask, then delete a group; its hosts move to Ungrouped.
pub fn confirm_delete_group(
    window: &mut Window,
    cx: &mut App,
    model: Entity<AppModel>,
    name: String,
) {
    if name.is_empty() {
        return;
    }
    confirm(
        window,
        cx,
        ConfirmOptions::new("Delete Group", delete_group_message(&name))
            .confirm_label("Delete")
            .danger(),
        move |yes, window, cx| {
            if !yes {
                return;
            }
            if let Err(e) = model.update(cx, |m, cx| m.delete_group(&name, cx)) {
                toast(
                    window,
                    cx,
                    format!("Could not delete the group: {e}"),
                    ToastKind::Error,
                );
            }
        },
    );
}

/// Move a host to a group, reporting a failure.
pub fn move_to_group(
    window: &mut Window,
    cx: &mut App,
    model: &Entity<AppModel>,
    host_id: i64,
    group: &str,
) {
    if let Err(e) = model.update(cx, |m, cx| m.set_host_group(host_id, group, cx)) {
        toast(
            window,
            cx,
            format!("Could not move the host: {e}"),
            ToastKind::Error,
        );
    }
}

/// Write every host to a JSON file the user picks. Never a secret: stored
/// URIs carry none, and passwords stay in the keyring.
pub fn export_hosts(window: &mut Window, cx: &mut App, model: &Entity<AppModel>) {
    let json = match model.read(cx).export_hosts() {
        Ok(json) => json,
        Err(e) => {
            toast(window, cx, format!("Export failed: {e}"), ToastKind::Error);
            return;
        }
    };
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64);
    file_dialog::save_file(
        window,
        cx,
        "Export hosts",
        &export_file_name(now),
        Some(JSON),
        move |path, window, cx| {
            if let Err(e) = std::fs::write(&path, json) {
                toast(window, cx, format!("Export failed: {e}"), ToastKind::Error);
            }
        },
    );
}

/// Import hosts from an exported JSON file the user picks.
pub fn import_json(window: &mut Window, cx: &mut App, model: Entity<AppModel>) {
    file_dialog::pick_file(
        window,
        cx,
        "Import hosts",
        Some(JSON),
        move |path, window, cx| {
            let result = (|| -> Result<_, String> {
                let size = std::fs::metadata(&path).map_err(|e| e.to_string())?.len();
                if size > MAX_IMPORT_BYTES {
                    return Err("too large".into());
                }
                let text = std::fs::read_to_string(&path).map_err(|e| e.to_string())?;
                parse_hosts_file(&text)
            })();
            let raw = match result {
                Ok(raw) => raw,
                Err(e) if e == "too large" => {
                    toast(
                        window,
                        cx,
                        "That file is too large to be a host export",
                        ToastKind::Error,
                    );
                    return;
                }
                Err(e) => {
                    toast(window, cx, format!("Import failed: {e}"), ToastKind::Error);
                    return;
                }
            };
            // One bad port fails the file, as the Tauri command's
            // deserialization does: nothing is written.
            let entries = match raw
                .iter()
                .map(|h| {
                    normalize_import_host(h).map(|host| ImportEntry {
                        host,
                        replace_id: None,
                    })
                })
                .collect::<Result<Vec<_>, _>>()
            {
                Ok(entries) => entries,
                Err(e) => {
                    toast(window, cx, format!("Import failed: {e}"), ToastKind::Error);
                    return;
                }
            };
            match model.update(cx, |m, cx| m.import_hosts(entries, cx)) {
                Ok(summary) => {
                    let (message, tone) = summarize_import(&summary, 0);
                    toast(window, cx, message, summary_toast(tone));
                }
                Err(e) => toast(window, cx, format!("Import failed: {e}"), ToastKind::Error),
            }
        },
    );
}
