# TermDrop

> A fast, native SSH client and SFTP browser built with [Tauri](https://tauri.app/) v2.

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
3. **Host manager** — grouped host list with search
4. **Settings panel** — theme and font size options
5. **Keyboard shortcuts** — help overlay
-->

## Features

### SSH Terminal
- **Multi-tab** terminal powered by [xterm.js](https://xtermjs.org/) with full ANSI color support
- **In-terminal search** (`Ctrl + F`)
- **Copy, paste, select-all** shortcuts (`Ctrl + Shift + C / V / A`)
- **Automatic reconnect** with SSH keep-alive on connection loss

### SFTP Browser
- **Visual file browser** with breadcrumb navigation
- **Upload, download, rename, delete** files and folders
- **Create folders** and **recursive delete**
- **Sort** by name, size, or modified date
- **Drag & drop** file uploads
- **Copy remote path** to clipboard
- **Progress bars** for active transfers

### Host Management
- **Save and organize** unlimited SSH hosts
- **Groups** and **favorites** for quick access
- **Search** hosts by name, address, username, or group
- **Password** or **SSH key** authentication
- **Import / export** host list as JSON
- **OS keyring** password storage — secure and local

### UI & Customization
- **Light & dark** theme support
- **Adjustable** terminal font size
- **Resizable** sidebar and panels
- **Keyboard shortcuts** help overlay
- **Toast notifications** for errors and successes

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

- [Node.js](https://nodejs.org/) 18+
- [Rust](https://rustup.rs/) stable

### Run locally

```bash
npm install
npm run tauri dev
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
| `Ctrl + Shift + V` | Paste from clipboard |
| `Ctrl + Shift + A` | Select all |

### Tabs

| Shortcut | Action |
|----------|--------|
| `Ctrl + Tab` | Next tab |
| `Ctrl + Shift + Tab` | Previous tab |
| `Ctrl + W` | Close active tab |

### SFTP

| Shortcut | Action |
|----------|--------|
| Double-click | Open folder |
| Right-click | File actions |
| Drag & drop | Upload files |

> Press `?` or click the help icon to see the shortcuts overlay anytime.

---

## Security

- **Passwords are stored in the OS keyring only** — never in the local SQLite database
- **No cloud sync**, no telemetry, no analytics
- **All data stays local** on your machine
- See [`SECURITY.md`](SECURITY.md) for vulnerability disclosure

---

## Contributing

Issues and pull requests are welcome!

- Read [`SECURITY.md`](SECURITY.md) before reporting security issues
- See [`CHANGELOG.md`](CHANGELOG.md) for release history
- Licensed under [MIT](LICENSE)

---

## License

[MIT](LICENSE) © sevheng
