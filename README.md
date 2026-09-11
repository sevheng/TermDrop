# TermDrop

> A fast, native SSH client, SFTP browser and datastore console built with
> [Tauri](https://tauri.app/) v2.

[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)
[![Tauri](https://img.shields.io/badge/Tauri-v2-24C8DB?logo=tauri)](https://tauri.app/)
[![Vue](https://img.shields.io/badge/Vue-3-4FC08D?logo=vue.js)](https://vuejs.org/)
[![Rust](https://img.shields.io/badge/Rust-000000?logo=rust)](https://www.rust-lang.org/)

**Windows** · **macOS** · **Linux**

---

<!--
## Screenshots

> TODO: Add screenshots

Recommended captures:
1. **Terminal view** — multi-tab SSH session with ANSI colors
2. **SFTP browser** — file list with breadcrumbs and sorting
3. **MongoDB browser** — database tree beside the document table
4. **Redis browser** — keyspace tree with a type-aware value view
5. **Host manager** — grouped host list with search
6. **Docker panel** — container list with controls
7. **Settings panel** — appearance and font size options
8. **Keyboard shortcuts** — help overlay
-->

## Features

### SSH Terminal
- **Multi-tab** terminal powered by [xterm.js](https://xtermjs.org/) with full ANSI color support
- **In-terminal search** (`Ctrl + F`)
- **Copy and select-all** shortcuts (`Ctrl + Shift + C / A`); paste from the
  right-click menu
- **Reconnect** button when a session drops, restoring the terminal and its SFTP session

### SFTP Browser
- **Visual file browser** with breadcrumb navigation
- **Upload, download, rename, delete** files and folders
- **Create folders** and **recursive delete**
- **Sort** by name, size, or modified date
- **Drag & drop** file uploads
- **Copy remote path** to clipboard
- **Progress bars** for active transfers

### MongoDB Browse, Backup & Restore
- **Browse** — click a collection to read its documents beside the tree, as a
  table or as JSON, with a filter, sort and pagination. Read-only
- **Backup** — dump selected collections to a compressed folder or a single
  archive file
- **Restore** — load a backup folder or archive back into the connection
- **Database-level** checkbox to select all collections at once
- **Bundled MongoDB Database Tools** — no external installation required

### Redis Browse, Backup & Restore
- **Browse** — databases and their key counts on the left, keys grouped by their
  `:` prefix. A `MATCH` pattern and a type filter narrow the `SCAN` itself, not
  just the list on screen. Read-only: there is no command console and no edit path
- **Type-aware viewers** for strings, hashes, lists, sets, sorted sets and
  streams, each showing TTL, encoding and size
- **Nothing can stall a server** — every read is a `SCAN` variant or an explicit
  range, never `KEYS`, `HGETALL`, `SMEMBERS` or `LRANGE key 0 -1`, so a
  ten-million-element set opens as fast as an empty one and a 5 MB string is
  previewed rather than fetched whole
- **Backup and restore** to a `.tdredis` file, per key via `DUMP`/`RESTORE` — so
  it works on managed Redis where `SYNC` and `BGSAVE` are blocked. TTLs are kept
  as remaining time, and a truncated or damaged file is refused rather than
  half-restored
- **Optional SSH tunnel** — reach a Redis that listens only on a private network
  through an SSH host already saved in TermDrop
- **Binary-safe keys** — a key that is not valid UTF-8 is carried as its exact
  bytes and shown as base64, so it can still be opened, backed up and restored

### Host Management
- **Save and organize** unlimited SSH hosts, MongoDB and Redis connections
- **Groups** and **favorites** for quick access
- **Search** hosts by name, address, username, or group
- **Password** or **SSH key** authentication
- **Import / export** host list as JSON
- **OS keyring** password storage — secure and local

### Port Forwarding
- **Local port forward** — tunnel remote services to local ports
- **Dynamic SOCKS proxy** — browser-through-SSH support

### Server Management (via SSH)
- **Docker** container list with start/stop/restart/logs/shell access
- **Security audit** — automated checks for common misconfigurations
- **System stats** — CPU, memory, disk, network, and process overview

### UI & Customization
- **Light & dark** themes, chosen in Settings and previewed as you pick — the
  terminal follows the app, so a light window no longer frames a black terminal
- **Bundled typefaces** — IBM Plex Sans for the interface and JetBrains Mono for
  the terminal, so the app looks the same on every platform rather than falling
  back to whatever the OS supplies
- **Readable everywhere** — every piece of text meets the WCAG AA contrast
  standard on every background, in both themes
- **Keyboard navigable** — arrow keys move through the host list, focus is
  visible wherever it lands, and dialogs keep Tab inside them and close on Escape
- **Adjustable** terminal font size
- **Resizable** sidebar and panels
- **Keyboard shortcuts** help overlay
- **Toast notifications** for errors and successes
- **Auto-updater** — check for and install new releases automatically

---

## Download

Pre-built bundles are available on [GitHub Releases](https://github.com/sevheng/TermDrop/releases):

| Platform | Format |
|----------|--------|
| Windows | `.msi` installer |
| macOS | `.dmg` disk image |
| Linux | `.AppImage` or `.deb` package |

> No releases yet? Build from source below.

---

## Development

### Prerequisites

- [Node.js](https://nodejs.org/) 24 (see `.nvmrc`)
- [Rust](https://rustup.rs/) stable

On Linux, Tauri also needs:

```bash
sudo apt install libwebkit2gtk-4.1-dev libappindicator3-dev librsvg2-dev patchelf
```

### Run locally

```bash
npm install
npm run tauri dev
```

### Checks

The same gate CI runs on every push and pull request:

```bash
npm run lint && npm test          # eslint, then vitest
cd src-tauri
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
```

### Build

```bash
npm run tauri build
```

Output:
- Windows: `src-tauri/target/release/bundle/msi/*.msi`
- macOS: `src-tauri/target/release/bundle/dmg/*.dmg`
- Linux: `src-tauri/target/release/bundle/appimage/*.AppImage` or `*.deb`

---

## Keyboard Shortcuts

### Terminal

| Shortcut | Action |
|----------|--------|
| `Ctrl + F` | Find in terminal |
| `Ctrl + Shift + C` | Copy selection |
| `Ctrl + Shift + A` | Select all |
| `Esc` | Close the find bar |
| Right-click | Copy, paste and terminal actions |

### Tabs

| Shortcut | Action |
|----------|--------|
| `Ctrl + Tab` or `Ctrl + PageDown` | Next tab |
| `Ctrl + Shift + Tab` or `Ctrl + PageUp` | Previous tab |
| `Ctrl + W` | Close active tab |

### Host List

| Shortcut | Action |
|----------|--------|
| `↑` / `↓` | Move through hosts |
| `Home` / `End` | First / last host |
| `PageUp` / `PageDown` | Move a page at a time |
| `Enter` | Connect to the focused host |
| `Delete` | Delete the focused host |

### SFTP

| Shortcut | Action |
|----------|--------|
| Double-click | Open folder |
| Right-click | File actions |
| Drag & drop | Upload files |

### MongoDB Panel

| Shortcut | Action |
|----------|--------|
| Click a collection | View its documents |
| Click DB checkbox | Select/deselect all collections for a backup |
| Ctrl/Cmd+Enter | Run the query, in the document view |

### Redis Panel

| Shortcut | Action |
|----------|--------|
| Click a database | Scan its keyspace |
| Click a key | Show its value, TTL and encoding |
| `MATCH` box | Narrow the scan itself, not just the list on screen |

### Global

| Shortcut | Action |
|----------|--------|
| `Ctrl + Shift + ?` | Show keyboard shortcuts help |
| `Ctrl + Tab` or `Ctrl + PageDown` | Next tab |
| `Ctrl + Shift + Tab` or `Ctrl + PageUp` | Previous tab |
| `Ctrl + W` | Close active tab |
| `Esc` | Close the active dialog |

---

## Security

- **Passwords never touch the local SQLite database** — they go to the OS
  keyring, or to an AES-GCM encrypted file when no keyring is available
- **Backups and host exports carry no passwords**
- **No cloud sync**, no telemetry, no analytics
- **All data stays local** on your machine
- See [`SECURITY.md`](SECURITY.md) for vulnerability disclosure

---

## Contributing

Issues and pull requests are welcome!

- Read [`CONTRIBUTING.md`](CONTRIBUTING.md) to get started
- Read [`SECURITY.md`](SECURITY.md) before reporting security issues
- See [`CHANGELOG.md`](CHANGELOG.md) for release history
- Licensed under [MIT](LICENSE)

---

## License

[MIT](LICENSE) © sevheng
