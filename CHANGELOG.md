# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## [Unreleased]

## [0.2.0] — 2025-06-09

### Added
- **Docker Integration** — browse containers, start/stop/restart, view logs, and exec into containers via a bottom xterm panel
- **System Monitor Panel** — expandable status bar showing processes, network interfaces, and disk usage
- **Live Network Speed** — real-time download/upload rates in the status bar
- **Security Audit Panel** — automated security checks (SSH config, firewall, failed logins, updates, disk space) with per-host caching
- **SFTP Inline Editor** — edit remote text files directly in-app with local download/upload
- **SFTP Preview & Bulk Actions** — file preview pane, multi-select with context menu bulk operations
- **SFTP Directory Download** — download entire folders as tar.gz archives
- **SFTP Filter & Sort** — search and sort the remote file list
- **GitHub Actions Release Workflow** — automated builds for Windows, macOS (Intel + Apple Silicon), and Linux on tag push

### Changed
- Rebranded from "SSH Client" to "TermDrop"
- Renamed app identifiers: package.json, Cargo.toml, Tauri config
- Updated local database and keyring service names to `termdrop`
- Docker panel uses silent background refresh to avoid loading spinner flicker

### Performance
- Batched `get_system_stats` from 9 sequential SSH execs into 1 (6–8× faster)
- All blocking SSH commands now run in `tokio::task::spawn_blocking`
- Stale-while-revalidate cache for `docker_ps` (5s fresh, 15s stale, background refresh)
- Request coalescing for concurrent `docker_ps` calls
- Security audit runs in background on connect with per-host caching

### Fixed
- PTY session ID mismatch causing "PTY session not found" errors
- Stale timer crash when closing docker pane
- Blank terminal on new tab activation
- u8 overflow in security score calculation

## [0.1.0] — 2024

### Milestone W4 — UI Polish & Enhanced Features
- Host table view with multi-select and bulk operations
- Enhanced dialogs (Confirm, Prompt) with keyboard support
- Settings panel improvements
- Keyboard shortcuts help overlay
- Resizable panels
- Status dots for connection state
- SFTP details view and sorting
- Recursive delete with confirmation
- Terminal search (Ctrl+Shift+F)
- Global toast notifications
- SFTP mkdir, copy remote path
- Host search and empty state

### Milestone W3 — Settings & Stability
- Settings storage (font size, theme, download path)
- Auto-reconnect on connection loss
- SSH keep-alive
- Toast notifications for errors
- Tauri bundle configuration (MSI, DMG, AppImage, DEB)

### Milestone W2 — SFTP Browser
- SFTP file browser with file list and breadcrumb
- Upload, download, delete, rename file operations
- Progress bars for transfers
- Context menus
- Multi-tab terminal switching

### Milestone W1 — MVP
- Multi-tab SSH terminal powered by xterm.js
- Host CRUD management
- OS keyring password storage
- SQLite persistent storage
- Pinia state management
