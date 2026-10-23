//! Two jobs: the Windows icon and version info (`embed_windows_resources`),
//! and the MongoDB tools (`copy_mongo_tools`, below).
//!
//! Copies the bundled `mongodump` / `mongorestore` next to the native binary,
//! as `src-tauri/build.rs` does for the Tauri one, so the MongoDB tab's backup
//! and restore find them through `termdrop_core::mongodb`'s
//! `resolve_mongo_tool` (the executable's directory, then its parent, then
//! `PATH`) in `cargo run` and `cargo test` builds alike.
//!
//! The source is the Tauri app's `src-tauri/binaries/<os>-<arch>/`, so there
//! is one copy of the tools in the repository. Unlike the Tauri build, a
//! missing pair is a warning, not a failure: the native app still builds,
//! and backup then falls back to whatever `mongodump` is on `PATH`.

use std::path::{Path, PathBuf};
use std::{env, fs};

fn main() {
    embed_windows_resources();
    copy_mongo_tools();
}

/// On Windows, the executable's icon and version details (Explorer, the
/// taskbar, Task Manager, the Start menu shortcut the installer makes all show
/// them). The icon is the Tauri app's generated `icon.ico`. A failure is a
/// warning: the app still builds, with the generic icon.
fn embed_windows_resources() {
    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let icon = "../../src-tauri/icons/icon.ico";
    println!("cargo:rerun-if-changed={icon}");
    let mut res = winresource::WindowsResource::new();
    res.set_icon(icon)
        .set("ProductName", "TermDrop Native Preview")
        .set("FileDescription", "TermDrop Native Preview")
        .set("CompanyName", "TermDrop")
        .set("LegalCopyright", "MIT License");
    if let Err(e) = res.compile() {
        println!("cargo:warning=Could not embed the Windows icon and version info: {e}");
    }
}

fn copy_mongo_tools() {
    let target = env::var("TARGET").unwrap_or_default();
    let Some((platform, ext)) = platform_dir(&target) else {
        println!(
            "cargo:warning=No bundled MongoDB tools for target '{target}'; backup and \
             restore will use mongodump/mongorestore from PATH."
        );
        return;
    };

    let manifest = PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
    let src_dir = manifest.join("../../src-tauri/binaries").join(platform);

    // OUT_DIR is <target-dir>[/<triple>]/<profile>/build/<pkg>-<hash>/out, so
    // the profile directory, where the binary lands, is three levels up. This
    // holds whatever CARGO_TARGET_DIR is, which an env lookup would not.
    let out = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR"));
    let Some(profile_dir) = out.ancestors().nth(3) else {
        println!("cargo:warning=Unexpected OUT_DIR layout: {}", out.display());
        return;
    };

    for name in ["mongodump", "mongorestore"] {
        let file = format!("{name}{ext}");
        let src = src_dir.join(&file);
        println!("cargo:rerun-if-changed={}", src.display());
        if !src.exists() {
            println!(
                "cargo:warning=Bundled MongoDB tool not found: {}; {name} will come from PATH.",
                src.display()
            );
            continue;
        }
        let dest = profile_dir.join(&file);
        if is_current(&src, &dest) {
            continue;
        }
        if let Err(e) = fs::copy(&src, &dest) {
            println!("cargo:warning=Could not copy {}: {e}", src.display());
            continue;
        }
        set_executable(&dest);
    }
}

/// `src-tauri/binaries/` directory and executable suffix for a target.
fn platform_dir(target: &str) -> Option<(&'static str, &'static str)> {
    Some(match target {
        "x86_64-unknown-linux-gnu" => ("linux-x86_64", ""),
        "aarch64-unknown-linux-gnu" => ("linux-aarch64", ""),
        "x86_64-apple-darwin" => ("macos-x86_64", ""),
        "aarch64-apple-darwin" => ("macos-aarch64", ""),
        // The GNU triple too: the native app is cross-built from WSL with
        // MinGW.
        "x86_64-pc-windows-msvc" | "x86_64-pc-windows-gnu" => ("windows-x86_64", ".exe"),
        "aarch64-pc-windows-msvc" => ("windows-aarch64", ".exe"),
        _ => return None,
    })
}

/// Skip the copy when the destination already has the same size and is not
/// older, so a no-op build does not rewrite ~30 MB of tools.
fn is_current(src: &Path, dest: &Path) -> bool {
    let (Ok(s), Ok(d)) = (fs::metadata(src), fs::metadata(dest)) else {
        return false;
    };
    s.len() == d.len() && matches!((s.modified(), d.modified()), (Ok(sm), Ok(dm)) if dm >= sm)
}

#[cfg(unix)]
fn set_executable(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    if let Ok(meta) = fs::metadata(path) {
        let mut perms = meta.permissions();
        perms.set_mode(0o755);
        let _ = fs::set_permissions(path, perms);
    }
}

#[cfg(not(unix))]
fn set_executable(_path: &Path) {}
