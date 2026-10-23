# Right-panel contract

The right panel beside an SSH tab has four tabs: **SFTP | Tunnels | Docker |
Security**, as in `src/views/MainWindow.vue`. The frame around them is done:
the strip, the resizable width (`sftp-width`, shared with the Tauri app), the
selected tab (in memory, SFTP at launch, one choice for all SSH tabs, as
Vue's `rightPanelTab`), when each panel is built and when it is dropped.
Tunnels shows a "not in the native app yet" state.

Each of the other three is one folder here with one entry point, already
registered in `DefaultPanels`:

```rust
// panels/sftp/mod.rs, panels/docker/mod.rs, panels/security/mod.rs
pub fn build(ctx: PanelContext, window: &mut Window, cx: &mut App) -> AnyView
```

**Edit only inside your own folder** (add files under it freely), plus at
most one dependency line in `native/app/Cargo.toml`. Everything shared is
listed below. If something you need is missing from it, say so rather than
editing `mod.rs`, `common.rs`, `tabs/` or `shell/`: the other two panel
agents are working at the same time.

## `PanelContext`

```rust
#[derive(Clone)]
pub struct PanelContext {
    pub kind: PanelKind,                 // Sftp | Tunnels | Docker | Security
    pub host_id: i64,                    // always an SSH host
    pub tab_id: Option<String>,          // Some only for the per-tab SFTP panel
    pub sftp_session_id: Option<String>, // Some only (and always) for SFTP
    pub services: Arc<Services>,         // .core: Arc<Core>, .rt: tokio Runtime, .bridge
    pub model: Entity<AppModel>,         // hosts, tabs, theme, font size, settings store
    /* private: the event hub, the open terminals, the insert confirm flag */
}

impl PanelContext {
    pub fn core(&self) -> &Arc<Core>;
    pub fn events(&self) -> Events;                       // for sftp_upload/download, exec_pty_connect
    pub fn spawn<F>(&self, f: F) -> tokio::task::JoinHandle<F::Output>      // async core call
        where F: Future + Send + 'static, F::Output: Send + 'static;
    pub fn spawn_blocking<T, F>(&self, f: F) -> tokio::task::JoinHandle<T> // sync work off the UI thread
        where F: FnOnce() -> T + Send + 'static, T: Send + 'static;
    pub fn subscribe<V: 'static>(&self, key: EventKey, view: WeakEntity<V>,
        handler: impl Fn(&mut V, &CoreEvent, &mut Context<V>) + 'static) -> PanelSubscription;
    pub fn new_pty_session_id(&self) -> String;             // "exec-<uuid>"
    pub fn terminal_palette(&self, cx: &App) -> termdrop_terminal::Palette;
    pub fn is_shown(&self, cx: &App) -> bool;               // this panel is on screen now
    pub fn host_name(&self, cx: &App) -> Option<String>;
    pub fn active_session_id(&self, cx: &App) -> Option<String>;
    pub fn insert_command(&self, command: &str, window: &mut Window, cx: &mut App)
        -> Result<(), InsertRefused>;                      // NoShell | NotInsertable
    pub fn show_terminal_pane(&self, pane: AnyView, cx: &mut App) -> Option<String>; // tab id
    pub fn close_terminal_pane(&self, tab_id: &str, cx: &mut App);
}

pub enum EventKey {
    SftpTransfer(String), // CoreEvent::SftpProgress for this sftp session id
    ExecPty(String),      // PtyConnected / PtyDisconnected / PtyError of this exec PTY id
}
```

`PanelSubscription` unsubscribes when dropped: keep it in your view's state.
Events arrive on the UI thread in the order the core emitted them; a
subscription whose view was released removes itself.

### Running core calls

Every `Core` method that touches the network blocks or is async; neither
may run on the UI thread. The pattern the terminal tabs use
(`tabs/ssh.rs`):

```rust
let core = ctx.core().clone();
let host_id = ctx.host_id;
let job = ctx.spawn(async move { core.docker_ps(host_id, true).await });
self.task = Some(cx.spawn_in(window, async move |this, cx| {
    let result = job.await.unwrap_or_else(|e| Err(e.to_string()));
    let _ = cx.update(|window, cx| {
        this.update(cx, |this, cx| this.loaded(result, window, cx))
    });
}));
```

Keep the `Task` in the view (dropping it cancels the UI half; the tokio half
still runs to the end). Errors are `String`s with the exact Tauri wording;
show them with `crate::ui::toast::toast(window, cx, msg, ToastKind::Error)`
or in the panel, as the Vue panel did.

### Being on screen

A panel is built the first time it is shown and then kept (the Vue
`KeepAlive`), so `build` runs once per tab (SFTP) or per host (Docker,
Security). To react to being shown or hidden, observe the model and ask:

```rust
cx.observe(&ctx.model, |this: &mut MyPanel, _, cx| {
    let shown = this.ctx.is_shown(cx);
    if shown != this.was_shown { /* start / stop polling */ }
})
```

### Shared helpers

- `panels::common`: `format_bytes`, `format_rate`, `format_speed`,
  `percent`, `format_permissions(perm, is_dir)`, `format_file_date(mtime)`
  (local time, "Oct 23" / "Oct 23, 2024"), `format_civil_date`,
  `format_elapsed(secs)` ("just now", "12s ago", "5m ago", "3h ago"),
  `base_name`, `parent_dir`, `join_remote`, and the `listStyles.js` values:
  `text_2xs`, `list_head`, `list_row(e, p, selected, interactive)`,
  `row_separator`, `list_cell`, `list_cell_muted`, `num`.
- `termdrop_core::sftp_ops::shell_escape` for any path or name put in a
  remote command (Vue's `utils/shell.js`; the same quoting, except that
  the core also quotes `~` and leaves non-ASCII letters bare).
- `crate::ui`: `EmptyState` (`empty` / `filtered` / `loading`: keep the three
  distinct, and an empty state offers its fix), `PanelHeader`, `IconButton`
  (requires a label), `dialogs::{confirm, prompt}`, `toast`, `file_dialog`
  (native open/save dialogs), `theme::palette(cx)`.
- Colours are tokens only (`palette(cx).ink_2`, `.line`, `.accent`...),
  never literals. A filled control takes `accent_solid` / `bad_solid`, never
  the bright tone under a white label. Text sizes are the named tiers
  (`text_xs`, `common::text_2xs`), never a bare pixel size without leading.

## Lifetimes

| Panel | Keyed by | Built | Dropped |
|---|---|---|---|
| SFTP | the tab | first shown, once the tab's SFTP session is up (the frame shows "Connecting SFTP..." or the failure with Retry until then) | the tab closes |
| Docker | the host | first shown | the host's last SSH tab closes |
| Security | the host | first shown | the host's last SSH tab closes |
| Tunnels | the host | (not ported) | |

"The host's last SSH tab closes" is exactly when the core drops the host's
exec session, its Docker and security caches and its exec PTYs
(`ssh_disconnect` -> `forget_host_state`), and when the Vue store drops its
security report. State a per-host panel keeps in its own view therefore dies
at the right moment without any extra code.

The SFTP session is the tab's own SSH connection, opened in the background
after the shell connects (`sftp_connect`), reconnected under the **same id**
by `ssh_reconnect`, and closed with the tab (`sftp_disconnect`). Never call
`sftp_connect` or `sftp_disconnect` from the panel.

## SFTP (`sftp/`)

Port of `SftpPanel.vue`, `FloatingEditor.vue`, `FilePreviewDialog.vue`,
`composables/useSftpTransfers.js`.

`let id = ctx.sftp_session_id.clone().unwrap();` then, all on `ctx.core()`
(`termdrop_core::sftp_ops`):

- `sftp_realpath(&id, ".".into())` for the home directory (what Vue starts at).
- `sftp_list(&id, path) -> Vec<SftpFile>` (dirs first, by name; no `.`/`..`).
- `sftp_upload(ctx.events(), &id, local_path, remote_path)` and
  `sftp_download(ctx.events(), &id, remote_path) -> local path` (into the
  Downloads folder, like Tauri), or `sftp_download_to(events, &id, remote,
  local)` after a save dialog. Progress: subscribe to
  `EventKey::SftpTransfer(id)` and read `CoreEvent::SftpProgress { file,
  bytes_transferred, total_bytes, .. }` (`file` is the remote path;
  throttled to ~100 ms / 1 %, with a final event at the total).
- `sftp_download_dir(&id, remote) -> local dir`: tars on the host over the
  host's exec session, so it needs the shell connected.
- `sftp_read_file` / `sftp_read_file_base64` (preview), `sftp_stat_file`
  (size + mtime, to detect a file changed under the editor before saving),
  `sftp_write_file` (atomic: temp file + rename, mode kept),
  `sftp_delete`, `sftp_rename`, `sftp_mkdir`, `sftp_rmdir` (recursive).
- `sftp_ops::write_file(path, content)` for a local "save as".

Calls on one SFTP session are serialised by the core (one mutex per session),
so a large transfer delays a listing, as in the Tauri app.
The column choice is the `sftp-columns` setting (`model.read(cx).store()`'s
`get_setting` / `set_setting`, JSON as `usePrefs.js` writes it).

## Docker (`docker/`)

Port of `DockerPanel.vue` plus the logs/exec pane in `TerminalTab.vue`.

On `ctx.core()` (`termdrop_core::exec_ops`), all over the host's shared exec
session (one command at a time, held under its mutex with a 60 s timeout):

- `docker_ps(host_id, all) -> Vec<Container>`, cached per host (fresh 5 s,
  served stale and refreshed behind up to 15 s).
- `docker_start` / `docker_stop` / `docker_restart(host_id, container_id)`
  and `docker_install(host_id)` (each drops the cached listing).
  `container_id` goes into the remote command unquoted: pass only ids that
  came from `docker_ps`.
- `docker_inspect_shell(host_id, id) -> "bash" | "sh"`.
- Errors: test with `contains` for `termdrop_core::docker::{DOCKER_NOT_INSTALLED,
  DOCKER_PERMISSION_DENIED, DOCKER_DAEMON_NOT_RUNNING}`, as Vue does.
- Poll every 5 s **only while `ctx.is_shown(cx)`**: the Vue panel stops on
  deactivation, because a hidden poll holds the host's exec session.

Logs / exec pane (the Vue `docker-pane-open` flow):

1. `let pty = ctx.new_pty_session_id();` and
   `let sub = ctx.subscribe(EventKey::ExecPty(pty.clone()), pane.downgrade(), ...)`
   **before** connecting; handle `PtyConnected`, `PtyDisconnected` (an exec
   pane closes itself; a logs pane stops "Following") and `PtyError`.
2. Build a `termdrop_terminal::TerminalView` (see `tabs/terminal.rs`) with
   `palette: ctx.terminal_palette(cx)` and the model's font size; its input
   closure calls `core.exec_pty_write_bytes(&pty, bytes)` (Vue batches
   keystrokes on a 16 ms timer; optional). There is no exec PTY resize.
3. `core.exec_pty_connect(ctx.events(), host_id, pty.clone(), command).await`
   with the Vue commands: `docker logs [-f] --tail 200 <name>` and
   `docker exec -it <name> <shell>`, names through `shell_escape`.
4. Then `ctx.services.bridge.attach_exec_pty(&core, &pty, view.read(cx).feed())`:
   it replays output that arrived before the attach, so nothing is lost.
5. `ctx.show_terminal_pane(pane_view.into(), cx)` puts the pane under the
   active terminal and returns its tab id for `close_terminal_pane`. The tab
   owns the pane from then on; hold only a `WeakEntity`. End the PTY in the
   pane's own release hook (`cx.on_release`): `core.exec_pty_disconnect(&pty)`
   (via `spawn_blocking`) and `bridge.detach_exec_pty(&pty)`. That runs when
   the pane is replaced, closed or its tab closes.

## Security (`security/`)

Port of `SecurityPanel.vue` and the store's `securityReports`.

- `core.run_security_audit(host_id, force) -> SecurityReport`, cached per host
  (fresh 30 s, stale up to 300 s; `force` skips it). `generated_at` is the
  run's Unix time: the age label is `common::format_elapsed(now -
  generated_at)`, refreshed every 15 s while shown.
- **Lazy**: nothing runs on connect. Start the audit from `build` (building
  is the panel being opened) once per host; never retry an error by
  itself; Refresh calls with `force: true`. The report lives in the view,
  which is dropped with the host's last SSH tab, exactly when Vue dropped it.
- **Remediation goes through `ctx.insert_command(cmd, window, cx)` and
  nothing else.** It checks that the active tab is a connected shell on this
  host, rejects control characters and anything over 512 characters, trims,
  writes with **no trailing newline**, focuses the terminal, and asks once
  per run first. Never call `ssh_write` yourself. The commands come from the
  report (`check.remediation`), which the core builds only from its own
  literals; never build one from report text.

## Checks

From `native/`, with the linker flags in `COMPARE.md`:

```bash
cargo fmt --check
cargo clippy --offline --all-targets -- -D warnings
cargo test --offline
```

Put pure logic (parsing, sorting, labels) in plain functions with
`#[cfg(test)]` tests in your folder; views are not unit tested.

Live runs: never against the real `~/.local/share/termdrop.db` or `~/.ssh`.
Set `HOME` and `XDG_DATA_HOME` to fresh scratch directories, use the local
containers from `CLAUDE.md` (`docker-compose.*.yml`), and type nothing into a
remote shell that changes it beyond files you created under `/tmp`.
