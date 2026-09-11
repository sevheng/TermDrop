# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## [Unreleased]

### Added
- **The interface has been tidied throughout.** Every list, table and empty state now shares one treatment instead of fifteen, numbers line up in columns, and the host list fits more on screen without dropping anything.
  - **Empty states tell you what to do.** They now distinguish "nothing here yet" from "nothing matched your filter" from "still loading" — and offer the button that fixes it. The opening screen offers to add your first host instead of only telling you to pick one
  - **The host list shows more.** Favourited hosts show their star without hovering, the host whose tab is open is marked, Redis and MongoDB hosts are colour-coded, and hosts you have connected to before show when you last did
  - **The keyboard works.** Focus is visible everywhere it lands, and dialogs keep Tab inside them and close on Escape

- **The shortcuts overlay lists the host-list keys.** Arrow-key navigation of the host list shipped with the interface pass but nothing announced it, so `Ctrl + Shift + ?` now shows it alongside the terminal, tab and SFTP shortcuts.

- **Dropdowns are drawn by the app now.** The native control could not be themed — its list was a separate platform surface that ignored the app's colours entirely, and on Linux its text was close to unreadable. The replacement matches the rest of the interface in both themes and keeps the keyboard behaviour: arrows and Home/End to move, Enter to choose, Escape to dismiss, and type a few letters to jump.

- **A light theme, and a way to choose it.** Settings now has an Appearance control beside the font size, previewed live as you pick it. The terminal follows the app, so a light window no longer frames a black terminal.

- **Redis connections.** A Redis server is a host row of its own, like a MongoDB connection, and opens its own tab: browse the keyspace, and back it up or restore it.
  - **Read-only key browser.** Databases and their key counts on the left, with keys grouped by their `:` prefix; a `MATCH` pattern box and a type filter narrow the scan itself, not just the list on screen. Type-aware viewers for strings, hashes, lists, sets, sorted sets and streams, showing TTL, encoding and size. Nothing in the browser can write: there is no command console and no edit path
  - **Nothing here can stall a server.** Every read is a `SCAN` variant or an explicit range — never `KEYS`, `HGETALL`, `SMEMBERS` or `LRANGE key 0 -1` — so opening a ten-million-element set costs the same as opening an empty one, and a 5 MB string is previewed rather than fetched whole
  - **Backup and restore** to a `.tdredis` file, per key via `DUMP`/`RESTORE`. This works on managed Redis where `SYNC` and `BGSAVE` are blocked, works through a tunnel, and can back up a single database or a single `MATCH` pattern. TTLs are preserved as remaining time, so a key dumped with an hour left has an hour left when it is restored
  - A truncated or damaged backup is **refused rather than half-restored** — the file carries a record count and a checksum, both verified before the first key is written — and a restore the target's Redis version cannot read is refused up front, naming both versions, instead of failing partway through
  - Restoring will not overwrite by default: a key that already exists is counted as skipped, and the summary says how many. Emptying the database first is a separate, opt-in checkbox
  - **Optional SSH tunnel.** A Redis that only listens on a private network can be reached through an SSH host already in TermDrop. The tunnel is bound to `127.0.0.1` only, opens and closes with the tab, and reports a bad credential or a bastion with forwarding disabled when it opens rather than on the first command
  - Passwords are kept in the OS keyring and never written to the database or into a backup file, the same as MongoDB's
  - Keys that are not valid UTF-8 stay addressable: they are carried as their exact bytes and shown as base64, so a binary key can be opened, backed up and restored rather than silently mangled
  - Backing up a server in **cluster mode is refused**, because `SCAN` only sees the node it is connected to and the result would be a silently partial backup

- **MongoDB document browser** — double-click any collection in the database tree, or use the browse icon on its row, to read its documents. Documents show as a **table** whose columns are the fields found on the page, so several documents can be compared at a glance; a toggle switches to the JSON view, and clicking a row expands the full document either way. A JSON filter and sort, pagination, expandable pretty-printed documents, per-document copy, and collection size and index count in the header. Read-only: there are no write or aggregation commands, so nothing typed here can modify the database.
  - A 24-character hex `_id` is treated as an ObjectId, so the common shorthand matches instead of silently returning nothing
  - Values are shown without losing precision — large integers and `Decimal128` keep their exact value rather than being rounded through a floating-point number
  - An unfiltered count is read from collection metadata rather than scanning, so opening a very large collection stays fast
  - Query errors appear under the filter box rather than as a toast that disappears while you are still editing

### Changed
- **A new app icon.** The old one was the right idea — a droplet with a shell prompt — drawn in a way that fell apart where it is actually seen. At 32px, the size the taskbar and dock use, the prompt closed up and the whole thing read as a blue blob; the near-black tile dissolved into a dark dock; and the glossy droplet belonged to no other part of the app. It is now drawn as a terminal window holding the droplet, in the app's own accent blue, on a tile with a rim that stays visible on any background.
  - **Two drawings, not one.** A window inside a tile inside a mark cannot survive 32px, so sizes at or below 64px use a chrome-free variant — the droplet enlarged to fill the tile, the prompt knocked straight out of it — while 128px and up get the full mark. Both are hand-authored SVG, committed, and regenerated by `npm run icons`
  - The macOS icon is no longer soft: the `.icns` was being built in a legacy pre-Retina format and now carries a modern PNG member for every slot, up to 1024px
  - The browser tab and devtools showed the Vite logo, left over from the project scaffold. They show TermDrop now

- **The interface has been repainted.** Cooler, deeper neutrals with more separation between panels, and a brighter accent that works as text rather than only as a button. Every piece of text in the app now meets the WCAG AA contrast standard on every background, in both themes — secondary text used to fall below it, and it was the second most-used colour in the app.
  - **The app has a typeface.** Nothing set one before, so the interface rendered in whatever each OS happened to supply. It is now IBM Plex Sans, bundled with the app rather than fetched
  - **Terminals are readable everywhere.** The terminal asked for Menlo or Monaco, which exist only on macOS, so Linux and Windows fell through to Courier New. It is now JetBrains Mono, which ships with the app
  - Modals sit visibly above the panel they cover instead of at the same level, and destructive buttons use a red dark enough for their white label to be legible

- **A MongoDB connection is now one connection.** A host used to hold a "remote" and a "local" URI so the two could be synced; it now holds one, and the panel offers **Browse**, **Backup** and **Restore** against it.
  - **Sync has been removed.** A configured second connection is not discarded: on first launch it becomes its own host, named `"<name> (local)"`, taking its stored password with it
  - The panel is now **two panes** — databases on the left, documents on the right. Clicking a collection shows its data immediately; ticking it includes it in a backup
  - No more direction toggle, no `Remote`/`Local` labels, and no mode chip

- **MongoDB connections are pooled and time-limited.** Expanding a database no longer opens a fresh connection each time, and an unreachable host now fails in seconds instead of hanging on the driver's 30-second default. Sync, dump and restore remain unbounded.

### Fixed
- **MongoDB dumped the whole database when only some collections were selected.** Only a single-collection selection was filtered, so selecting two of forty collections dumped all forty.
- **Dumping several databases to an archive kept only the last one**, while reporting success for each. An archive holds one database, and selecting more than one is now refused with a pointer to the folder dump.
- **MongoDB dump and restore ignored the direction toggle**, always using the remote connection even when the panel was showing local collections. Both now act on the connection whose tree is selectable and name it on the button, and a restore refreshes that tree so the restored databases appear without reopening the tab.
- **Restoring always dropped the target collections.** `--drop` was hardcoded; it is now a checkbox that defaults to off. **This changes existing behaviour**: a restore now merges and reports duplicate `_id` conflicts unless the box is ticked.
- **A sync that fell back to the driver silently lost indexes**, collection options and validators, and still reported plain success. The fallback now warns before it runs, says so in the result, recreates indexes, and can be refused.
- **MongoDB progress was invented from elapsed time.** It now reports the progress mongodump and mongorestore actually print, falling back to an estimate only for the first few seconds, before the tools emit any.
- **Cancelling a multi-database MongoDB operation could be ignored** between two databases.
- A panic while holding a MongoDB lock disabled every later MongoDB operation until restart, and a tool process could be orphaned rather than killed.

- **Security audit reported passes it could not verify** — four of the eight checks could return a false pass on a stock RHEL 9 or Ubuntu host, so an insecure server could score 87 "Good". Checks that cannot determine an answer now report `unknown` and are excluded from the score rather than counting as passes, and the panel shows how many checks passed out of how many could be determined.
  - `ufw status` reporting `inactive` matched a `contains("active")` test and was read as active
  - an unset `PasswordAuthentication` or `PermitRootLogin` matched a `contains("no")` test and was read as disabled; absence is now judged against the OpenSSH default
  - the SSH config is read with `sshd -T` where possible, so settings in `/etc/ssh/sshd_config.d/` are seen; Ubuntu 22.04+ and RHEL 9 keep the real values there
  - the SSH port check parses numbers instead of substring-matching `22`
  - the iptables probe counted blank lines, so an empty ruleset looked configured
  - dnf/yum security updates were structurally always zero
  - an unreadable auth log was reported as "no failed login attempts"
  - the sudo-user enumeration always returned a pass and is now informational
- **SOCKS5 proxy connected to the wrong host** for requests carrying a literal IP address, which affected curl, proxychains, and most CLI tools.
- **SFTP editor discarded unsaved changes** when a preview or a second file was opened, did not notice that the file had changed on the server, and truncated the target before writing, so an interrupted save could leave a half-written file.
- **SFTP panel stayed broken after a reconnect**, and directory listing failures were invisible.
- **A file dropped on the SFTP panel uploaded to every open host**, and transfer progress from one host appeared in every panel.
- **Docker panel kept polling while hidden**, reported a stopped daemon as a permissions problem, and installed Docker without asking.
- **Host import misreported its results** — a failed row was reported as a total failure even though earlier rows had been saved, and those rows did not appear until the list was reloaded.

### Security
- **MongoDB passwords are no longer stored in the database.** They move to the OS keyring (or the existing encrypted-file fallback) on first launch and the stored URI keeps only the username. Existing rows are migrated automatically; a keyring failure leaves the row untouched and retries next launch rather than losing the password.
  - Host **exports no longer contain MongoDB passwords**, and importing an older export strips them instead of writing plaintext back
  - The connection string is no longer written to the log file. `termdrop::mongodb=debug` is no longer on by default, and connection strings in command arguments and tool output are redacted
  - Credentials are no longer passed on the mongodump/mongorestore command line, where any local user could read them from `ps`. They go in a `--config` file created 0600 and deleted when the operation ends
  - MongoDB connection strings are no longer rendered into tooltips or list rows; only host and port are shown

## [0.2.6] — 2026-06-15

### Fixed
- **MongoDB dump/restore on macOS and in test builds** — copy bundled `mongodump`/`mongorestore` next to the compiled executable during the Tauri build and search the Cargo profile parent directory so tools are found from `target/debug` and `target/debug/deps`
- **MongoDB root-user authentication** — automatically add `authSource=admin` to connection strings that contain credentials but no explicit `authSource`, fixing auth failures during dump, restore, and sync

### Added
- Unit tests for bundled MongoDB tool resolution and URI normalization

## [0.2.5] — 2026-06-11

### Fixed
- **MongoDB cross-platform tools** — bundle platform-specific `mongodump` / `mongorestore` binaries (Linux x86_64, macOS x86_64 + aarch64, Windows x86_64) so dump/restore/sync work on every OS instead of shipping a Linux binary everywhere

### Changed
- **MongoDB panel UX**
  - Single full-width database tree when only a remote URI is configured (dump/restore mode)
  - Cleaner two-pane headers in sync mode with compact `Remote`/`Local` labels and `From`/`To` badges
  - Direction pill in the header and a larger center swapper for bidirectional sync

## [0.2.4] — 2026-06-11

### Added
- **MongoDB Sync & Dump/Restore** — new host capability for MongoDB databases
  - Side-by-side Remote/Local database tree view with bidirectional sync toggle
  - Sync selected collections via bundled `mongodump`/`mongorestore` CLI tools
  - Dump to BSON files (with gzip compression) when no local URI is configured
  - Restore from BSON files back to remote
  - Database-level checkbox to select all collections at once
  - Bundled MongoDB Database Tools — no external installation required
- `MongoDbModal` for adding standalone MongoDB connections (no SSH required)
- `DbTree` shared component for database tree rendering

### Changed
- Host sidebar now routes MongoDB-only hosts directly to MongoDB panel on click
- MongoDB-only hosts show Database icon and cleaned-up URI subtitle
- Removed separate "Open MongoDB" button from host list — click row directly

## [0.2.3] — 2025-06-10

### Added
- **Tauri Auto-Updater** — automatic update checks on startup, manual check in Settings, download progress UI, and one-click install & relaunch
- Update signing with minisign keypair
- GitHub Releases integration for serving update manifests

## [0.2.2] — 2025-06-10

### Fixed
- **Windows SSH handshake** — added `openssl-on-win32` feature to force OpenSSL crypto backend on Windows, fixing "Unable to exchange encryption keys" (LIBSSH2_ERROR_KEX_FAILURE)

## [0.2.1] — 2025-06-10

### Added
- **SSH Config Import** — parse `~/.ssh/config` and import host entries with one click
- **TermDrop App Icon** — custom icon with teardrop + terminal prompt symbol for all platforms

### Performance
- **Terminal Rendering** — switched from WebGL to Canvas renderer for stable tab switching
- **Buffered I/O** — smart input routing and 4KB/16ms output batching for smoother terminal feel
- **Binary Data Channel** — raw `Vec<u8>` IPC channel instead of JSON events for lower latency

### Fixed
- Blank terminal when switching between tabs (Canvas context loss)
- "Unable to exchange encryption keys" on Windows (enabled `vendored-openssl`)

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
