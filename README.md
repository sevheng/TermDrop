# SSH Client

A lightweight SSH client and SFTP browser built with Tauri v2 and Vue 3.

## Features

- **Multi-tab SSH terminal** powered by xterm.js with ANSI support
- **SFTP file browser** with upload, download, delete, rename, and progress bars
- **Host configuration** management with OS keyring password storage
- **Persistent settings** for font size, theme, and download path
- **Automatic reconnect** on connection loss with keep-alive

## Tech Stack

- **Shell:** Tauri v2 (Rust)
- **UI:** Vue 3 + TailwindCSS + Pinia
- **Terminal:** xterm.js + xterm-addon-fit
- **SSH/SFTP:** ssh2 crate
- **Storage:** SQLite + OS keyring

## Prerequisites

- [Node.js](https://nodejs.org/) 18+
- [Rust](https://rustup.rs/) stable

## Development

```bash
npm install
npm run tauri dev
```

## Build

```bash
npm run tauri build
```

Output:
- Windows: `src-tauri/target/release/bundle/msi/*.msi`
- macOS: `src-tauri/target/release/bundle/dmg/*.dmg`
- Linux: `src-tauri/target/release/bundle/appimage/*.AppImage` or `*.deb`

## Security

- Passwords are stored in the OS keyring only — never in SQLite
- No cloud sync or telemetry
- All data stays local
