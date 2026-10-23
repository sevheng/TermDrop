# Tauri vs native: how to run and compare them

The native app (GPUI Kit, this folder) is being built beside the Tauri app,
not instead of it yet. This page says how to run each one, what the native one
can and cannot do today, and how to compare them fairly. The plan behind it is
`docs/gpui-migration-plan.md` (outside the repo, one level up).

## Running them

Both apps open the same SQLite database, `<data_dir>/termdrop.db`
(`~/.local/share/termdrop.db` on Linux), through `termdrop-core`. Hosts,
groups, favourites, theme, font size, pane widths and trusted host keys are
shared, and passwords come from the same keyring entries. Run them one at a
time if you are editing hosts: neither app watches the database for changes
made by the other.

### Tauri app

From `ssh-client/` (on `master` or on this branch, the Tauri app is the same):

```bash
npm install
npm run tauri dev      # dev build, Vite on :1420, hot reload
npm run tauri build    # release bundle under src-tauri/target/release/bundle/
```

Compare against the **release** build (`src-tauri/target/release/termdrop`, or
the installed bundle), not `tauri dev`: dev mode serves an unminified frontend
through Vite and runs a debug Rust backend.

### Native app

```bash
cd native
cargo run --release              # the real app, on the shared database
cargo run --release -- --demo    # fictional hosts, in-memory settings, no DB/keyring/network
```

The binary is `native/target/release/termdrop-native`. `RUST_LOG` filters its
stderr log the same way as the Tauri app's.

It runs as **TermDrop Native Preview** (`com.termdrop.native-preview`), one
copy per user: a second launch brings the running window forward and exits.
The Tauri app is a different instance, so both can be open at once on the same
database (writes wait up to 5 s for each other's lock). The preview is also
released as installers that sit beside the Tauri app; see `RELEASING.md`.

On Linux it needs these packages (Ubuntu/Debian names):

```bash
sudo apt install libxkbcommon-x11-dev libx11-xcb-dev libzstd-dev libvulkan1
```

On a machine that has the runtime libraries but cannot install the `-dev`
packages, the link fails for want of the unversioned `.so` symlinks. Make a
directory of them and point the linker at it, for native builds only (setting
`RUSTFLAGS` for `src-tauri` forces a full Tauri rebuild):

```bash
mkdir -p ~/linkfix
L=/usr/lib/x86_64-linux-gnu
ln -sf $L/libX11-xcb.so.1       ~/linkfix/libX11-xcb.so
ln -sf $L/libxkbcommon-x11.so.0 ~/linkfix/libxkbcommon-x11.so
ln -sf $L/libzstd.so.1          ~/linkfix/libzstd.so
ln -sf $L/libvulkan.so.1        ~/linkfix/libvulkan.so
RUSTFLAGS="-L $HOME/linkfix" cargo run --release   # from native/
```

The native workspace is separate from `src-tauri` on purpose: its own
`Cargo.lock` and `target/`, so neither build disturbs the other.

## What the native app does today

Read from the code on this branch, not from the plan.

| Area | Native | Notes |
|---|---|---|
| Window, title bar, resizable sidebar and right pane | Yes | Widths are saved to the shared settings. The window's size, position and maximized state are saved to its own `native-window-state` setting and restored (on Linux the size and maximized state only: under X11 the restored position crept by the decoration size). |
| Fonts | Yes | IBM Plex Sans (UI), JetBrains Mono (terminal and code) and Symbols Nerd Font Mono are embedded and registered at startup, so they do not have to be installed. Nerd Font icons in the terminal come through the text system's own fallback, not the terminal font's fallback list: GPUI drops a fallback face that has no `m` glyph, which a symbols-only font never has. On Linux that works unless another font covering the Private Use Area sorts first (GNU Unifont does, and then the icons show as hex boxes); macOS and Windows are unchecked. |
| Host sidebar | Yes | Search, grouped/flat view, collapsible groups, favourites, keyboard navigation (Enter connects, Delete asks to delete). Header: add menu (host, MongoDB, Redis, remote desktop), import menu, export, local shell. Row hover buttons: favourite, edit, delete. Context menus: host (connect/open, open remote desktop, edit, favourite, copy name, delete, move to group), group (add host here, rename, delete), empty area (new group, add host). No drag-and-drop between groups yet. |
| Adding, editing, deleting hosts | Yes | One dialog per kind (SSH, MongoDB, Redis, VNC) with the Vue fields, defaults and messages, plus a group picker, a favourite box and an SSH host's own VNC settings. Saves go through the same `termdrop-core` operations as the Tauri commands (`hosts.rs`). Passwords go to the keyring; fields start empty and an empty one keeps what is stored. Deleting a host closes its tabs first. Where native is stricter than Tauri: a Redis username and password typed in the fields are percent-encoded into the URI (Tauri splices them in raw, so a password with `/`, `?` or `#` is not split out and lands in the database), a URI whose password still could not be split is refused, a row whose URI still holds its password stores that password before the row is rewritten, and saving over a host the other app deleted is an error rather than a silent no-op. |
| Import and export | Yes | `~/.ssh/config` (hosts already saved start unticked; `ProxyJump` linked after import, and only on rows that import created — Tauri can re-point an existing host with the same name, address and port when its re-import fails). A JSON port that is not a whole number fails the file, as in Tauri, with native's own wording. A JSON import keeps a row's Redis URI (password stripped) and VNC settings; Tauri's drops them, so an exported Redis- or VNC-only host fails its own re-import with "Port must be between 1 and 65535, got 0" and an SSH host loses its screen. JSON files are picked with native file dialogs (`rfd`, GTK on Linux; GTK is pointed at X11 when GPUI runs there, since with `WAYLAND_DISPLAY` unset it would otherwise open its dialogs on the default Wayland socket, invisibly to an X11 session). |
| Settings | Partly | Theme (dark/light) and terminal font size only. |
| Tabs | Yes | Tab strip, Ctrl+Tab / Ctrl+PageUp/Down, Ctrl+W with a confirm, Ctrl+Shift+T, Ctrl+, , Ctrl+Alt+B. |
| Local terminal | Yes | Shell detection and the shell menu from `termdrop-core`, reconnect. |
| SSH terminal | Yes | Through `termdrop-core`: host-key trust dialog (per hop, jump hosts), password prompt on a keyring miss, reconnect. |
| Terminal element (`termdrop-terminal`) | Yes | `alacritty_terminal` grid painted by GPUI: colours, selection, scrollback (1000 lines), mouse reporting, bracketed and chunked paste, URL open, OSC 52, title, bell. No ligatures. |
| Terminal search, WebGL/DOM renderer split | n/a / No | No search yet. GPUI always renders on the GPU. |
| Right panel frame | Yes | SFTP / Tunnels / Docker / Security strip beside SSH tabs, resizable, panels built on first show and kept per tab (SFTP) or per host. Each SSH tab opens its SFTP connection in the background, keeps it across a reconnect and closes it with the tab. Contract for the panels: `app/src/panels/README.md`. |
| SFTP panel | Yes | Per tab, on the tab's own SFTP connection (kept across a reconnect). Home directory first, path bar, sort, filter, Size/Modified/Perms columns (`sftp-columns`, shared with Tauri and followed live by every open panel), multi-select, upload from a dialog or by dropping files, download (files to Downloads, folders tarred on the host), mkdir, rename, delete, transfer progress. Floating editor (drag, resize, wrap, Ctrl+S, asks before discarding or overwriting a file changed on the server) drawn over the app and under its dialogs; preview for text (Find), images and binaries. Differs from Tauri: upload and mkdir at `/` build `/name` (Tauri built a relative path, landing in the home directory), a failed transfer is marked Failed instead of sticking, the toolbar Upload reports errors, the preview's Find counts and steps through matches rather than matching lines (a line with three hits counts three), a folder download's archive goes to a unique temp file instead of `Downloads/<folder>.tar.gz`, a control character in a remote name (a newline, ESC) is shown as `?` in the list, path bar, transfers, titles and prompts, as `ls` does (the webview collapsed a newline to a space and drew ESC as nothing). The preview starts at 640 x 480 and resizes from its bottom-right handle down to 320 x 200, as Vue's did (checked live: 640 x 480, dragged to 840 x 630, then pinned at 320 x 200). Like Vue's it grows about its horizontal centre; unlike Vue's, its top stays put rather than it growing about its vertical centre (GPUI's dialog is anchored a tenth of the way down), and it is capped at the window less a margin rather than 90vw x 80vh. |
| Docker panel | Yes | Per host, over the shared exec session. Container list (Show all, 5 s poll only while shown), start/stop/restart with the Vue confirms, install, and the not-installed / daemon-stopped / permission-denied states with their commands. Logs (`-f` for running containers) and exec open a pane under the terminal, closed with the shell. No exec PTY resize, as in Tauri. Tested live only as far as "not installed" (no Docker on the test container); the list, actions and pane are unit tested and read from the code. |
| Security audit | Yes | Per host and lazy (runs when the panel is first opened, dropped with the host's last SSH tab). Score, grade, checks, "What to do" with Copy and Send to terminal (typed with no newline, after a one-time confirm, only into a connected shell on that host). |
| Port forwards (Tunnels) | **No** | The panel says it is not ported. |
| MongoDB tab | Yes | One tab per host (opening again activates it; closing the host's last MongoDB tab drops its pooled client). Database tree with lazy collections, three-state database ticks and the "N selected · clear" line; documents with filter/sort checked as you type, Run, count-then-page (25/50/100/200), Table (`_id` first, the common fields, 12 columns) and JSON views, expandable rows with ObjectId/ISODate/Binary written out as Vue did, Copy, and the stats line; an Indexes view (`mongodb_list_indexes`, which Vue never showed). Backup to a folder or an archive and restore from a folder or an archive, through the bundled `mongodump`/`mongorestore`, with the Vue confirms, wording, file filters (Gzip archive, BSON archive, All files) and per-run progress with Cancel. Differs from Vue: closing the tab cancels a running backup or restore (Vue's carried on with nothing to show it), open databases reload their collections after a refresh, a database whose ticks are all removed leaves the selection, error states have Retry. Tested live (read-only): tree, documents, JSON, Indexes, ticking, an archive backup of two collections. Restore not run against a server (it writes); its confirm and job planning are unit tested. |
| Redis tab | Yes | One tab per host (the last tab's close drops the host's pooled connections and its tunnel). Connect with the Redis and the tunnel's SSH password prompts, error banner with Reconnect, "via <bastion>" and "cluster" badges; database tree with prefix groups; key list by `SCAN` pages (pattern, type filter, Sizes, prev/next by cursor, pages never trimmed, keys addressed by `key_b64`, binary keys marked); value view for string (256 KB preview), hash, list, set, sorted set and stream with Load more; backup to `.tdredis` and restore with the Vue dialog (db picker, Overwrite, Empty first). Differs from Vue: Load more stops at a list's or sorted set's total, long cells and string lines are drawn in parts (the page is never trimmed), a string has Copy value, an empty finished scan offers Clear filter / Refresh, closing the tab cancels a running backup or restore. The tree's "No keys on this server" / "Every database is empty" state is not ported because Vue never reaches it: `RedisPanel.vue` substitutes `db0` when `INFO keyspace` lists no databases, so an empty server shows `db0  0` and "No keys match" in both apps (`databases_or_default`, checked live against the empty `redis-private`). Tested live (read-only): `redis-local` and the tunnelled `redis-private`, an empty backup written and its header read back by the restore dialog, a truncated file refused with its offset. Both servers were empty, so the key list and value view with data are unit tested only. |
| Remote desktop (VNC) | Yes | One tab per host, for a VNC-only host or "Open remote desktop" on an SSH host (its last VNC tab's close runs `vnc_disconnect_host`, compared by kind as well as host, so the shell beside it does not hold the tunnel open). Through `termdrop-core` (`vnc_ops.rs`, the Tauri commands' own bodies): tunnel through the host's own or another SSH host, the Vue header (address, size, "tunnelled", "view only", the `vnc.showStats` readout), Refresh, View only, Fit / Actual size (the shared `vnc.fitMode`), Ctrl+Alt+Delete, Send clipboard (with the Latin-1 warning), remote clipboard and bell only from the tab in front, error banner with Reconnect. Frames are parsed, JPEG-decoded and applied on a thread per tab into a BGRA framebuffer and drawn as 128-pixel GPU tiles, only the changed ones rebuilt; each frame is acknowledged once applied, a hidden tab or window pauses the stream and withholds the ack, and showing it again resumes with a full refresh. Every app shortcut (Tab, Ctrl+W, Ctrl+Tab, Ctrl+C...) goes to the remote while the screen has focus; Ctrl+Alt+Shift hands the keyboard back; held keys are released on blur and when the window loses focus. Differs from Vue: a missing VNC password (none stored and the server rejects the empty one) and a tunnel's missing SSH password are asked for and stored (Vue showed the error; a VNC answer the server rejects is forgotten, so Reconnect asks again), a cleanly ended session shows Disconnected with Reconnect (Vue went blank), 1:1 mode pans with the pointer instead of showing scrollbars, the remote cursor shape (Cursor pseudo-encoding) is painted as a sprite over the screen at the pointer, hotspot on the pointer and unscaled as a CSS cursor is, but the local arrow stays visible above it (GPUI has neither an image cursor nor a hidden one), and a remote that hides its pointer hides only the sprite; none is painted in view only, where Vue showed the default cursor (checked live on `:5903`: the arrow over the desktop and the I-beam over the xterm follow the pointer, go when it moves into the letterbox, and go in view only), modifiers are sent as left-hand keys and the keypad as the main row (GPUI reports neither side nor keypad), a symbol typed with Shift and a composed character are sent as a press and release at once (their release cannot be matched to the press), input-method text is typed a character at a time. Tested live: TigerVNC (`:5901`, password prompt), LibVNCServer (`:5902`), no-auth (`:5903`), and `vnc-private` through the `:2223` bastion (SSH and VNC password prompts, the fail-closed untrusted-key error, then trust from an SSH tab); drawing, scaling, 1:1 panning, clicks, typing with Shift symbols, Tab completion, Ctrl+C and Ctrl+W reaching the remote, Ctrl+Alt+Shift, bell toasts, Tight JPEG rectangles. Second live run (2026-10-23, every server seeded by `scripts/seed-vnc.sh`): picture checked against `import`/`xwd` of the remote root after 1-2 minutes of window moves and typing on each server (pixel-exact at 1:1 on `:5903`), keys checked byte for byte through a raw `cat` in the remote xterm (letters, shifted symbols, Enter, Backspace, arrows, Tab, Ctrl+C, Escape, Home/End/Delete, F1, Page Up), pointer at all four corners in both modes, wheel one notch per click in both modes, view only, a held key released on a tab switch, clipboard both ways (with the Latin-1 warning), Refresh, pause of a hidden tab and of an unmapped window, the tunnel's lifetime across SSH and VNC tabs, the missing-password prompt, a wrong password, and a stopped server. Not run: ARD (needs a Mac), minimise (WSLg does not iconify; unmapping the window was used instead), Ctrl+Alt+Shift (synthetic modifier presses produce no GPUI event). |
| Desktop notifications | Partly | A finished MongoDB or Redis backup or restore posts one notification when the window is not focused, as Vue's `notifyIfUnfocused` does (GPUI's system notifications; on a desktop with no notification daemon, WSLg among them, it only logs). Nothing else notifies yet. |
| Packaging | Preview | `cargo-packager` (`[package.metadata.packager]` in `app/Cargo.toml`): macOS `.app` + `.dmg`, Windows NSIS installer (per user), Linux `.deb` + AppImage, all as "TermDrop Native Preview" beside the Tauri app, with `mongodump`/`mongorestore` inside. Released as GitHub prereleases from `native-v*` tags by `native-release.yml`; see `RELEASING.md`. Built and run locally on Linux only (deb 46 MB, AppImage 54 MB). |
| Single instance | Yes | Keyed apart from the Tauri app; a second launch focuses the first. Under WSLg the window manager ignores the focus request (the request arrives; `_NET_ACTIVE_WINDOW` from an application is not honoured). |
| Updater, tray | **No** | No auto-update in the preview by design (Phase 8's switch-over release is separate). |

So the fair comparison today is **terminal tabs, the SSH right panel**
(SFTP, Docker, Security) **and the MongoDB, Redis and remote-desktop tabs**. `--demo` has no backend, so
its tabs are placeholders and the right panel shows its frame only; run
against a scratch database (`XDG_DATA_HOME`, `HOME`) and a test host to see
the panels.

## Benchmarks

The two harnesses do not measure exactly the same span, so compare each app
against its own earlier runs first, and across apps with care.

### Tauri

```bash
VITE_TERM_PERF=1 npm run tauri dev                          # throughput, echo latency, idle tabs
VITE_TERM_PERF=1 VITE_TERM_PERF_ONLY=latency npm run tauri dev   # echo latency only, 400 samples
VITE_TERM_PERF=1 VITE_TERM_PERF_ONLY=app npm run tauri dev       # a real full-screen app (copane in WSL)
```

`src/dev/termPerf.js` opens real local tabs and writes a JSON report to
`<data_dir>/termdrop/perf/`. Throughput is `seq 1 2000000` (~14 MB) and
8 MB of base64, three runs each; latency is keystroke to the moment xterm.js
has *processed* the echo, with a per-stage breakdown from the backend's
`dev_perf::mark` stamps. It is dev-build only (Vite drops it from release), so
its absolute numbers include a debug backend. `VITE_TERM_PERF_ONLY=app` drives
copane (`ce`); see `VITE_TERM_PERF_DISTRO` / `VITE_TERM_PERF_APP` in the file.
CPU is not in the report: run `pidstat -p <pid> 1` beside it.

### Native

```bash
cd native
cargo run --release --example local_shell -p termdrop-terminal -- --bench
```

One window, one terminal, JSON on stdout:

- `seq`: `seq 1 1000000` through a real PTY (6.9 MB, 7.9 MB once the PTY
  turns each `\n` into `\r\n`), until the frame that shows its last byte is
  painted. Note: half the Tauri workload.
- `burst`: 17 MB of coloured log lines fed straight into the view, no PTY:
  parse + paint alone.
- `latency`: 200 synthetic keystrokes dispatched through the window into
  `cat`, timed to the **end of the paint** that shows the echo. That is later
  than the Tauri harness's end point (xterm processed, before its paint), so
  the native number is the stricter one; neither includes the compositor.
- `ctrl_c_reached_program`: checks that Ctrl+C goes to the program, not to a
  Copy binding.

Without `--bench` the example is just a terminal on `$SHELL`, which is the
quickest way to feel the element without the rest of the app.

### By hand

Release builds of both, same machine, same font size, window maximised:

- **Typing feel**: type fast in a local shell and in an SSH tab; hold a key
  for autorepeat; edit in `vim`/`htop`. Any lag or skipped frames?
- **Big output**: `seq 1 2000000`, `cat` a large log, `find /`. Time to
  finish, and is the window responsive (switch tabs, scroll) while it runs?
  Ctrl+C reaction time.
- **Many tabs**: open 10 local tabs, each running `top -d 0.5`. Switch
  between them; watch CPU.
- **Memory**: RSS after start with no tabs, and with 10 tabs.
  Linux: `ps -o rss,cmd -C termdrop-native` and, for Tauri, sum the app and
  its `WebKitWebProcess`/`WebKitNetworkProcess` children
  (`ps -o rss,cmd --ppid <pid>`). Windows: Task Manager, "Memory" column,
  with the WebView2 processes grouped under the app. macOS: Activity Monitor.
- **Startup time**: launch to first usable frame, cold (after a reboot) and
  warm.
- **Idle CPU**: one idle tab, then ten, with `pidstat`/Task Manager.

### Results

| Measure | Tauri | Native | Machine / notes |
|---|---|---|---|
| Echo latency, median (ms) | | | |
| Echo latency, p95 (ms) | | | |
| `seq 1 2000000`, seconds | | | |
| Longest UI stall during `seq` (ms) | | | |
| Idle CPU, 1 tab / 10 tabs (%) | | | |
| RSS at start, no tabs (MB) | | | |
| RSS with 10 tabs (MB) | | | |
| Startup to first frame, warm (ms) | | | |
| Startup, cold (ms) | | | |
| Typing feel (notes) | | | |
| Binary / bundle size (MB) | | | |

## Measured on WSL2 (dev machine)

### Remote desktop

2026-10-23, release build, `RUST_LOG=termdrop::vnc=debug` (the core's `VNC
frame timing` beside the native `VNC native render` line, once a second).
Same WSLg caveat as below.

| Server | Desktop | Steady state | Apply (parse + JPEG + framebuffer + tiles, off the UI thread) |
|---|---|---|---|
| no-auth `:5903` | 800x600, 1:1 | idle after the first second | 0.26 ms/frame over a 358-frame burst of 2 KB updates |
| TigerVNC `:5901` | 1280x800 fit | idle; ~260 updates/s for ~2 s after login | 0.33-0.39 ms/frame, 2-3 KB/frame |
| `vnc-private` via bastion | 1024x768 fit | 50 updates/s for a few seconds after waking, one Tight JPEG each | 0.18-0.38 ms/frame |

With 256-pixel tiles the login burst rebuilt ~3 tiles (750 KB of copying)
per 2 KB update; the tiles are now 128 pixels (64 KB each). Only the tiles
current at paint time are uploaded, so a burst faster than the display
uploads at most one set per frame. CPU with a connected idle desktop: 1-1.5%.

Second run, same machine, all four servers seeded. Update rates are the
core's `fps` on lines whose window was about one second; CPU is the whole
process from `/proc/<pid>/stat` over 10-20 s.

| Load | Server | Updates/s | Apply ms/frame | App CPU |
|---|---|---|---|---|
| Idle, no tabs | - | - | - | 0.4% |
| Idle, 1 VNC tab | TigerVNC | 0 | - | 1.4% |
| Idle, 4 VNC tabs | all | 0 | - | 1.8% |
| Pointer motion over a static desktop (~50 moves/s) | TigerVNC | 3-5 (no pixels change: the cursor is local) | 0.13-0.18 | 15% |
| Dragging a window through the app | TigerVNC | 51-136 | 0.8-2.6 | 64% |
| Dragging the photo window | `vnc-private` via bastion | 28-38 | 1.4-2.2 | 43% |
| Dragging a window | LibVNCServer | 9-11 (the server polls) | - | 10% |
| Windows moved and typed into by a script | TigerVNC | 230-330 | 0.2-0.35 | - |

During the TigerVNC drag the main thread was 37% and the render thread 10%.
TigerVNC sends a few hundred tiny one-JPEG updates after any change or
full refresh (probably its lossless refresh of JPEG areas), answered as fast as
the ack loop allows, and it is the same for the Tauri app.

2026-10-22, branch `gpui-kit`, release builds. Windows 11 host, WSL2 Ubuntu,
16 threads, AMD Radeon integrated graphics.

**Caveat: WSLg is not representative.** Its Vulkan device is `llvmpipe`
(software), so GPUI picked wgpu's GL backend over Mesa's D3D12 translation
instead. The plan (Phase 0) says not to judge the renderer here; these numbers
are a smoke test and a baseline for this machine, not the go/no-go figures.

### Native terminal benchmark

`cargo run --release --example local_shell -p termdrop-terminal -- --bench`,
three runs, grid 112x34:

| Run | `seq` s (MB/s) | `seq` longest stall ms | burst 17 MB s (MB/s) | burst longest stall ms | latency median / p95 / max ms |
|---|---|---|---|---|---|
| 1 | 0.796 (9.9) | 64.0 | 0.404 (44.1) | 61.2 | 0.94 / 1.33 / 28.5 |
| 2 | 0.733 (10.8) | 68.4 | 0.300 (59.5) | 37.5 | 1.00 / 1.42 / 29.0 |
| 3 | 0.770 (10.3) | 57.9 | 0.237 (75.1) | 41.6 | 0.94 / 1.30 / 28.9 |

- Latency is key dispatch to the end of the paint showing the echo, 200
  samples, no timeouts; it excludes compositor present time. The ~29 ms max
  appeared in every run, so it is probably one periodic frame wait rather
  than noise; worth a look on real hardware.
- During `seq` the view painted 16-19 frames (about 22 fps) and the longest
  single paint was 8-11 ms; the burst paints were 19-20 ms at most.
- Ctrl+C reached the program (`cat` ended) in all runs.

### Startup and memory

Measured by launching with `GDK_BACKEND=x11` and `WAYLAND_DISPLAY` unset, so
`xdotool` can see the window, polling every ~5 ms for a visible window titled
`TermDrop`, then reading RSS with `ps` 3 s later. Real database (the shared
one), no tabs open, warm cache.

| | Native (`termdrop-native`) | Tauri (`src-tauri/target/release/termdrop`, built 2026-10-17) |
|---|---|---|
| Launch to visible window, 4 / 3 runs (ms) | 466, 449, 427, 440 | 467, 168, 145 |
| RSS after 3 s (MB) | 207 (one process) | 224 app + 289 WebKit children = **513** |
| Release binary size (MB, not stripped) | 76 | 54 |

- The native window is titled during its first render, so its time is close
  to launch-to-first-frame. The Tauri window maps before the webview has
  painted (its own log puts "main window created" at 336-553 ms, after the
  X11 window was already visible), so its number is when an empty window
  appears, not the UI; the frontend reaches its first content later still.
  The two are not the same event and should not be read as "Tauri starts
  faster". On Windows/macOS, time it by hand to the first drawn host list.
- Native RSS is about 40% of Tauri's here. Part of the native figure is the
  GL driver stack mapped into the process, so recheck on real hardware.
- The Tauri binary predates `v0.5.1` by a few days; rebuild it
  (`npm run tauri build`) before comparing for real.
