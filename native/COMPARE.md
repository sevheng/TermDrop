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
| Window, title bar, resizable sidebar and right pane | Yes | Widths are saved to the shared settings. |
| Host sidebar | Yes | Search, grouped/flat view, collapsible groups, favourites, keyboard navigation (Enter connects, Delete asks to delete). Header: add menu (host, MongoDB, Redis, remote desktop), import menu, export, local shell. Row hover buttons: favourite, edit, delete. Context menus: host (connect/open, open remote desktop, edit, favourite, copy name, delete, move to group), group (add host here, rename, delete), empty area (new group, add host). No drag-and-drop between groups yet. |
| Adding, editing, deleting hosts | Yes | One dialog per kind (SSH, MongoDB, Redis, VNC) with the Vue fields, defaults and messages, plus a group picker, a favourite box and an SSH host's own VNC settings. Saves go through the same `termdrop-core` operations as the Tauri commands (`hosts.rs`). Passwords go to the keyring; fields start empty and an empty one keeps what is stored. Deleting a host closes its tabs first. Where native is stricter than Tauri: a Redis username and password typed in the fields are percent-encoded into the URI (Tauri splices them in raw, so a password with `/`, `?` or `#` is not split out and lands in the database), a URI whose password still could not be split is refused, a row whose URI still holds its password stores that password before the row is rewritten, and saving over a host the other app deleted is an error rather than a silent no-op. |
| Import and export | Yes | `~/.ssh/config` (hosts already saved start unticked; `ProxyJump` linked after import, and only on rows that import created — Tauri can re-point an existing host with the same name, address and port when its re-import fails). A JSON port that is not a whole number fails the file, as in Tauri, with native's own wording. A JSON import keeps a row's Redis URI (password stripped) and VNC settings; Tauri's drops them, so an exported Redis- or VNC-only host fails its own re-import with "Port must be between 1 and 65535, got 0" and an SSH host loses its screen. JSON files are picked with native file dialogs (`rfd`, GTK on Linux). |
| Settings | Partly | Theme (dark/light) and terminal font size only. |
| Tabs | Yes | Tab strip, Ctrl+Tab / Ctrl+PageUp/Down, Ctrl+W with a confirm, Ctrl+Shift+T, Ctrl+, , Ctrl+Alt+B. |
| Local terminal | Yes | Shell detection and the shell menu from `termdrop-core`, reconnect. |
| SSH terminal | Yes | Through `termdrop-core`: host-key trust dialog (per hop, jump hosts), password prompt on a keyring miss, reconnect. |
| Terminal element (`termdrop-terminal`) | Yes | `alacritty_terminal` grid painted by GPUI: colours, selection, scrollback (1000 lines), mouse reporting, bracketed and chunked paste, URL open, OSC 52, title, bell. No ligatures. |
| Terminal search, WebGL/DOM renderer split | n/a / No | No search yet. GPUI always renders on the GPU. |
| Right panel frame | Yes | SFTP / Tunnels / Docker / Security strip beside SSH tabs, resizable, panels built on first show and kept per tab (SFTP) or per host. Each SSH tab opens its SFTP connection in the background, keeps it across a reconnect and closes it with the tab. Contract for the panels: `app/src/panels/README.md`. |
| SFTP panel | Yes | Per tab, on the tab's own SFTP connection (kept across a reconnect). Home directory first, path bar, sort, filter, Size/Modified/Perms columns (`sftp-columns`, shared with Tauri and followed live by every open panel), multi-select, upload from a dialog or by dropping files, download (files to Downloads, folders tarred on the host), mkdir, rename, delete, transfer progress. Floating editor (drag, resize, wrap, Ctrl+S, asks before discarding or overwriting a file changed on the server) drawn over the app and under its dialogs; preview for text (Find), images and binaries. Differs from Tauri: upload and mkdir at `/` build `/name` (Tauri built a relative path, landing in the home directory), a failed transfer is marked Failed instead of sticking, the toolbar Upload reports errors, the preview's Find counts and steps through matches rather than matching lines (a line with three hits counts three), a folder download's archive goes to a unique temp file instead of `Downloads/<folder>.tar.gz`, a control character in a remote name (a newline, ESC) is shown as `?` in the list, path bar, transfers, titles and prompts, as `ls` does (the webview collapsed a newline to a space and drew ESC as nothing). No preview resize. |
| Docker panel | Yes | Per host, over the shared exec session. Container list (Show all, 5 s poll only while shown), start/stop/restart with the Vue confirms, install, and the not-installed / daemon-stopped / permission-denied states with their commands. Logs (`-f` for running containers) and exec open a pane under the terminal, closed with the shell. No exec PTY resize, as in Tauri. Tested live only as far as "not installed" (no Docker on the test container); the list, actions and pane are unit tested and read from the code. |
| Security audit | Yes | Per host and lazy (runs when the panel is first opened, dropped with the host's last SSH tab). Score, grade, checks, "What to do" with Copy and Send to terminal (typed with no newline, after a one-time confirm, only into a connected shell on that host). |
| Port forwards (Tunnels) | **No** | The panel says it is not ported. |
| MongoDB, Redis | **Frame only** | One tab per host and kind (opening again activates it); closing the host's last tab of that kind releases its pooled connections (and a Redis host's tunnel), as `closeServiceTab` does. The tab's content is a placeholder naming the host. The core calls, progress events and password prompt-and-retry the tabs will use are in place; contract: `app/src/datastores/README.md`. `app/build.rs` copies the bundled `mongodump`/`mongorestore` next to the binary. |
| VNC | **No** | Same notice. |
| Updater, packaging, tray/notifications, single instance | **No** | Phase 8. |

So the fair comparison today is **terminal tabs and the SSH right panel**
(SFTP, Docker, Security): local shells and SSH. `--demo` has no backend, so
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
