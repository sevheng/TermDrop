# Tauri v2 Native Features Integration Plan

## Current State

**Installed plugins:**
- `tauri-plugin-opener` — open files/URLs externally
- `tauri-plugin-dialog` — file picker dialogs
- `tauri-plugin-updater` — in-app updates
- `@tauri-apps/plugin-process` — JS-side only, not wired in Rust

**Capabilities configured:** `core:default`, `opener:default`, `dialog:default`, `updater:default`

---

## Recommended Features (prioritized by impact)

### 1. Window State Plugin — *High impact, trivial effort*
- Remembers window size, position, and maximized state across restarts
- One-line install: `npm run tauri add window-state`
- Zero config after adding `.plugin(tauri_plugin_window_state::Builder::new().build())`
- No frontend code needed — works automatically

### 2. Single Instance Plugin — *High impact, trivial effort*
- Prevents opening multiple app instances
- Focuses existing window when user tries to launch again
- One-line install: `npm run tauri add single-instance`
- Config: focus the main window in the init callback
- No frontend code needed

### 3. Clipboard Manager Plugin — *Medium impact, low effort*
- Native clipboard access (read/write text, images, files)
- More reliable than browser `navigator.clipboard` API
- Useful for: copying file paths from SFTP, copying terminal output
- Install: `npm run tauri add clipboard-manager`
- JS API: `readText()`, `writeText()`

### 4. Global Shortcut Plugin — *Medium impact, low effort*
- App-wide keyboard shortcuts even when app is not focused
- Useful for: quick-connect to favorite host, show/hide app, copy from terminal
- Install: `npm run tauri add global-shortcut`
- Register shortcuts in JS on app startup
- Permissions needed: `global-shortcut:allow-register`

### 5. Notification Plugin — *Medium impact, low effort*
- Native desktop notifications
- Useful for: long-running sync/dump completion, connection errors
- Install: `npm run tauri add notification`
- JS API: `sendNotification({ title, body })`
- Permissions needed: `notification:default`

### 6. Shell Plugin — *Medium impact, medium effort*
- Spawn child processes with proper stdin/stdout/stderr handling
- Could replace raw `std::process::Command` for mongodump/mongorestore
- Better error handling, streaming output, cross-platform path resolution
- Install: `npm run tauri add shell`
- Requires permission scopes for allowed commands

### 7. System Tray — *Low impact, medium effort*
- Tray icon with context menu
- Minimize-to-tray behavior
- No plugin needed — built into `tauri::Builder`
- Requires custom tray icon asset

---

## Approach Options

### Option A: "Quick Wins" (Recommended)
Install **Window State + Single Instance + Clipboard Manager**. These are 1-line installs, zero UI work, and immediately improve daily UX. Total effort: ~30 min.

**What changes:**
- `Cargo.toml` / `package.json` — add 3 plugin deps
- `main.rs` — init WindowState and SingleInstance plugins
- Capabilities JSON — add clipboard permission
- `App.vue` or entry point — optionally replace browser clipboard with native

### Option B: "Power User Pack"
Option A + **Global Shortcut + Notification**. Adds app-wide shortcuts and completion alerts for sync/dump. Total effort: ~1 hour.

**What changes (in addition to A):**
- Register shortcuts on app mount (e.g. `Ctrl+Shift+T` to show/hide)
- Emit notifications on sync/dump/restore completion
- Add notification settings in Settings panel

### Option C: "Full Native"
Option B + **Shell + System Tray**. Replaces process spawning with Tauri's native Shell API and adds tray icon. Total effort: ~2 hours.

**What changes (in addition to B):**
- Refactor `mongodb.rs` to use `tauri::api::process::Command` (Shell plugin)
- Add tray icon to `tauri.conf.json` bundle config
- Implement tray menu (show, hide, quit)
- Handle tray double-click to show window

---

## Files that would be modified

| File | Changes |
|------|---------|
| `src-tauri/Cargo.toml` | Add plugin crates |
| `src-tauri/src/main.rs` | Init plugins, configure callbacks |
| `package.json` | Add JS plugin packages |
| `src-tauri/capabilities/default.json` | Grant new permissions |
| `src/App.vue` or `src/main.js` | Register global shortcuts, clipboard |
| `src/stores/settings.js` | Add notification/shortcut preferences |
| `src/components/SettingsPanel.vue` | Toggle switches for new features |
| `src-tauri/src/mongodb.rs` | (Option C only) Use Shell plugin for spawning |
