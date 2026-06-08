# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## [Unreleased]

### Changed
- Rebranded from "SSH Client" to "TermDrop"
- Renamed app identifiers: package.json, Cargo.toml, Tauri config
- Updated local database and keyring service names to `termdrop`

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
