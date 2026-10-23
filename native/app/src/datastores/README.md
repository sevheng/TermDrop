# Datastore tab contract

A MongoDB-only or Redis-only host, and any host's remote desktop (VNC),
opens as a **tab** (not a right panel), one per host and kind, as `openServiceTab` / `closeServiceTab` in
`src/stores/connection.js` do. The frame is done:

- **Opening**: double-clicking the row (or its menu) calls
  `AppModel::activate_host` -> `open_service`, which activates the host's tab
  of that kind when there is one and otherwise opens `mongo-N` / `redis-N`.
- **Building**: `tabs::CoreContent::build` makes a `ServiceTabContext` and
  calls `DefaultDatastores::build`, which calls your folder's `build` **once**
  when the tab opens. The main window keeps the view until the tab closes
  (switching tabs keeps its state).
- **Closing**: Ctrl+W / the tab's x asks first, then `AppModel::close_tab`.
  `CoreContent::tab_closed` calls `mongodb_disconnect` / `redis_disconnect`
  / `vnc_disconnect_host` (in the core: drop the host's pooled client, or
  its pooled connections and its SSH tunnel, or its VNC sessions and
  tunnel) **only when that was the host's last tab of that kind** — kind as
  well as host id, `datastores::release_on_close` (a VNC tab sits beside the
  shell on an SSH host). Then the view is dropped. Never call these from a
  tab; a VNC tab closes only its own session (`vnc_disconnect`).

Each tab is one folder with one entry point:

```rust
// datastores/mongo/mod.rs, datastores/redis/mod.rs, datastores/vnc/mod.rs
pub fn build(ctx: ServiceTabContext, window: &mut Window, cx: &mut App) -> AnyView
```

All three are built: `mongo/` (tree, documents, indexes, backup and
restore), `redis/` (key tree, key list, value view, backup and restore) and
`vnc/` (the remote screen; see "Remote desktop" below). The placeholder
they started from is gone.

**Ownership:** each tab's own code stays in its folder. What both use, or
what a third tab would want, goes in `common.rs` (data shapes and drawing
helpers) or `crate::ui` (components with no datastore in them): write it
there rather than a private copy, and say so in the report.

## `ServiceTabContext`

```rust
#[derive(Clone)]
pub struct ServiceTabContext {
    pub kind: DatastoreKind,          // Mongo | Redis | Vnc
    pub tab_id: String,               // "mongo-3", "redis-4", "vnc-5"
    pub host_id: i64,                 // the MongoDB- or Redis-only host
    pub services: Arc<Services>,      // .core: Arc<Core>, .rt: tokio Runtime, .bridge
    pub model: Entity<AppModel>,      // hosts, tabs, theme, settings
    /* private: the event hub */
}

impl ServiceTabContext {
    pub fn core(&self) -> &Arc<Core>;
    pub fn events(&self) -> Events;                    // for dump/restore/export/import
    pub fn spawn<F>(&self, f: F) -> tokio::task::JoinHandle<F::Output>;     // Future + Send + 'static
    pub fn spawn_blocking<T, F>(&self, f: F) -> tokio::task::JoinHandle<T>; // FnOnce + Send + 'static

    // Run on the runtime, get the result back on the view (UI thread).
    pub fn run<V, T, Fut>(&self, window: &mut Window, cx: &mut Context<V>, future: Fut,
        done: impl FnOnce(&mut V, Result<T, String>, &mut Window, &mut Context<V>) + 'static,
    ) -> Task<()>;
    // The same, with the password prompt-and-retry (see below).
    pub fn run_with_secret<V, T, F, Fut>(&self, window: &mut Window, cx: &mut Context<V>,
        call: F,  // Fn(Arc<Core>, Option<String> /* ssh_password */) -> Fut + Send + Sync + 'static
        done: impl FnOnce(&mut V, Result<T, String>, &mut Window, &mut Context<V>) + 'static,
    ) -> Task<()>;

    // Backup / restore progress.
    pub fn new_op_id(&self) -> String;                 // "mongo-op-<uuid>" / "redis-op-<uuid>"
    pub fn subscribe_op<V: 'static>(&self, op_id: &str, view: WeakEntity<V>,
        handler: impl Fn(&mut V, &CoreEvent, &mut Context<V>) + 'static) -> OpSubscription;
    // A VNC session's events, plus what it emitted before the view could listen.
    pub fn subscribe_session<V: 'static>(&self, session_id: &str, view: WeakEntity<V>,
        handler: impl Fn(&mut V, &CoreEvent, &mut Context<V>) + 'static)
        -> (OpSubscription, Vec<CoreEvent>);
    pub fn cancel_op(&self, op_id: &str);              // mongodb_cancel / redis_cancel
    pub fn app_version(&self) -> &'static str;         // for redis_export's header

    // The host.
    pub fn host(&self, cx: &App) -> Option<HostSummary>;      // the sidebar row, no URI
    pub fn host_name(&self, cx: &App) -> Option<String>;
    pub fn host_record(&self, cx: &App) -> Result<Option<HostRecord>, String>; // full row, read now
    pub fn display_uri(&self, cx: &App) -> String;            // masked, for a header
    pub fn is_shown(&self, cx: &App) -> bool;                 // this tab is the active one
    pub fn close_tab(&self, cx: &mut App);                    // no confirm

    // Settings (the shared `settings` table) and the window.
    pub fn setting(&self, cx: &App, key: &str) -> Option<String>;
    pub fn set_setting(&self, key: &str, value: &str, cx: &mut App);
    pub fn toast(&self, window: &mut Window, cx: &mut App, message: impl Into<String>, kind: ToastKind);
    pub fn notify_if_unfocused(&self, window: &Window, cx: &App, title, body); // desktop notice
    pub fn pick_file(&self, window: &Window, cx: &App, title: &str, filters: &[Filter],
        on_pick: impl FnOnce(PathBuf, &mut Window, &mut App) + 'static);
    pub fn save_file(&self, window: &Window, cx: &App, title: &str, file_name: &str,
        filters: &[Filter], on_pick: impl FnOnce(PathBuf, &mut Window, &mut App) + 'static);
    pub fn pick_folder(&self, window: &Window, cx: &App, title: &str,
        on_pick: impl FnOnce(PathBuf, &mut Window, &mut App) + 'static);
}
```

`OpSubscription` unsubscribes when dropped: keep it in the view. Events
arrive on the UI thread in the order the core emitted them, only for your
kind (a Redis op never sees MongoDB events), and a subscription whose view
was released removes itself. Toast kinds are `ui::toast::ToastKind`
(`Info`, `Success`, `Warning`, `Error`; errors stay until dismissed). A
`Filter` is `("Redis backup", &["tdredis"])`; pass them in the Vue order,
the first being the one the dialog starts on, and end with
`ui::file_dialog::ALL_FILES` where Vue offered "All files" (rfd makes it
`*.*`, as the Tauri dialog plugin does).

`notify_if_unfocused` is Vue's `notifyIfUnfocused`: a desktop notification
only when the window is not focused (the toast covers the case where it
is), posted through GPUI's system notifications and never failing — WSLg
has no notification daemon and GPUI only logs that. Both tabs post one per
finished run ("Backup complete" / "Restore complete"); MongoDB's body comes
from `common::completed_summary`.

### Running calls

Every network call on `Core` is async (or blocking) and must not run on the
UI thread. Use `run`; keep the returned `Task` in the view (dropping it
cancels only the UI half — the core call still runs to its end):

```rust
let core = self.ctx.core().clone();
let (host, db) = (self.ctx.host_id, name.clone());
self.task = Some(self.ctx.run(window, cx,
    async move { core.mongodb_list_collections(host, db).await },
    |this, result, window, cx| this.collections_loaded(result, window, cx)));
```

Errors are `String`s with the Tauri app's exact wording; show them as the
Vue tab did (a toast, or a banner with Reconnect for the Redis connect
error).

### Passwords that are not stored

`run_with_secret` is the native `withMongoSecret` / `withRedisSecret`
(`composables/useSecretRetry.js`). It runs `call(core, None)`; when that
fails because a stored password is missing, it asks with the Vue wording
("MongoDB password required" / "Redis password required"), **stores** the
answer in the keyring, and runs `call` again. Each password is asked for at
most once per call (`common::next_secret_prompt`), so a tunnelled Redis host
missing both its own password and its bastion's gets two prompts in a row
rather than a raw keyring error after the first. Cancelling, an empty answer
or a second miss of the same password deliver the error. Use it for the calls the Vue tabs
wrap: `mongodb_list_databases`, `mongodb_list_collections`,
`redis_connect`.

Native tells apart what Vue did not (`common::missing_secret`): a Redis host
whose **tunnel's SSH host** has no stored password gets an "SSH password
required" prompt naming that host; the answer is stored as that SSH host's
password and passed as `ssh_password`, which only `redis_connect` takes.
So call it as:

```rust
self.ctx.run_with_secret(window, cx,
    move |core, ssh_password| async move { core.redis_connect(host_id, ssh_password).await },
    |this, result, window, cx| this.connected(result, window, cx))
```

For MongoDB ignore the second argument. Never put a password into a URI,
and never ask for one outside this path.

### Backup and restore

1. `let op = ctx.new_op_id();`
2. `self.sub = Some(ctx.subscribe_op(&op, cx.entity().downgrade(), |this, ev, cx| { if this.progress.apply(ev) { cx.notify() } }))`
   **before** starting.
3. `self.progress = common::OpProgress::new(op.clone(), "Backing up db0")`.
4. `ctx.run(window, cx, async move { core.redis_export(events, op, ...).await }, done)`;
   in `done`, `progress.finish(result.as_ref().map(|_| ()).map_err(String::as_str))`
   — `Err("cancelled")` (the core's exact string) is a cancel, not a failure.
5. Draw `common::op_progress_row(id, &progress, &palette, on_cancel)`;
   `on_cancel` calls `progress.cancelling()` and `ctx.cancel_op(&op)`.
6. Closing the tab does not stop a running operation by itself (nor did
   the Vue tab). Both tabs cancel it in the view's release hook
   (`cx.on_release(|this, _| this.ctx.cancel_op(&op))`), so a result never
   arrives with nothing left to show it.

Pick paths with `ctx.pick_folder` / `pick_file` / `save_file`; read local
files (`scan_restore_folder`, `redis::read_backup_header`) with
`spawn_blocking`.

## Core API

All on `ctx.core()` unless noted; `db`, `collection` and paths are owned
`String`s (so the future is `'static`). The Tauri commands are one-line
adapters over exactly these, so behaviour and error text match.

### MongoDB (`termdrop_core::mongo_ops`, types in `termdrop_core::mongodb`)

| Call | Returns | Notes |
|---|---|---|
| `mongodb_list_databases(host_id)` | `Vec<String>` | 30 s timeout. Wrap in `run_with_secret`. |
| `mongodb_list_collections(host_id, db)` | `Vec<String>` | 30 s. Wrap in `run_with_secret`. |
| `mongodb_find(host_id, db, coll, filter, sort, projection, skip, limit)` | `FindResult { documents: Vec<String>, truncated, elapsed_ms }` | `filter`/`sort`/`projection` are the user's JSON text (`Option<String>`, empty = none); each document is canonical extended JSON, field order kept (`common::json_lines`). `limit` capped at `mongodb::MAX_FIND_LIMIT` (200), page capped at 8 MB (`truncated`). Read-only: no aggregate, no writes. 60 s. |
| `mongodb_count(host_id, db, coll, filter)` | `CountResult { count, estimated }` | Unfiltered = metadata estimate. 60 s. |
| `mongodb_list_indexes(host_id, db, coll)` | `Vec<String>` | 30 s. |
| `mongodb_collection_stats(host_id, db, coll)` | `String` (JSON) | 30 s. |
| `mongodb_dump(events, op_id, host_id, db, collections, output_dir, is_archive)` | `()` | `mongodump`; empty `collections` = whole db; `is_archive` writes one archive file. No timeout. |
| `mongodb_restore(events, op_id, host_id, db, collections, input_dir, is_archive, drop_first)` | `()` | `mongorestore` from a dump folder (a dump root or one db's folder) or archive. |
| `mongodb_restore_archive(events, op_id, host_id, includes, input_path, drop_first)` | `()` | `includes` are `db.collection` namespaces. |
| `mongodb_cancel(&op_id)` | — | Via `ctx.cancel_op`. |
| `termdrop_core::mongodb::scan_restore_folder(path)` | `Vec<RestoreSourceDb { name, collections: [{ name }] }>` | Blocking: `spawn_blocking`. |

Events: `CoreEvent::MongoProgress(MongoProgress { op_id, db, collection,
stage, synced, total, percent, detail })` (a final `stage: "done"`), and
`CoreEvent::MongoCancelled { op_id, db }`.

### Redis (`termdrop_core::redis_ops`, types in `termdrop_core::redis`)

| Call | Returns | Notes |
|---|---|---|
| `redis_connect(host_id, ssh_password)` | `RedisServerInfo { version, mode, databases: [RedisDbInfo { index, keys, expires }], database_count, tunnelled, display_uri, supports_scan_type }` | Opens the tunnel (own SSH session) when the host has one; pools the URI's db. Wrap in `run_with_secret`. `display_uri` is masked: show it, never the stored URI. |
| `redis_server_info(host_id)` | `RedisServerInfo` | Re-read INFO after a backup/restore. |
| `redis_scan_keys(host_id, db, cursor, pattern, type_filter, limit, with_memory, supports_scan_type)` | `RedisScanPage { keys: [RedisKeySummary { key: RedisBytes, key_b64, kind, ttl_ms, memory_bytes }], cursor, done }` | `cursor` is a decimal string, `"0"` starts and ends. `limit` capped at `redis::MAX_KEYS_PER_PAGE` (200). The page may overshoot `limit`: **never trim it**. |
| `redis_key_tree(host_id, db, pattern)` | `KeyTree { groups: [PrefixGroup], scanned, truncated }` | Prefix groups from a scan capped at `redis::TREE_SCAN_CAP` keys / 3 s; say when `truncated`. |
| `redis_key_value(host_id, db, key_b64, cursor, offset, limit)` | `RedisValuePage { kind, entries: [RedisEntry { field, value, score }], total, ttl_ms, encoding, cursor, done }` | Hashes/sets page by `cursor` (`HSCAN`/`SSCAN`), lists/zsets by `offset`; strings are a capped prefix (`RedisBytes.truncated`). `limit` capped at `MAX_ELEMENTS_PER_PAGE` (200). |
| `redis_export(events, op_id, host_id, db, pattern, output_path, max_key_bytes, ctx.app_version())` | `RedisOpSummary` | `SCAN` + `DUMP` to `.tdredis`; keys over `max_key_bytes` (default `MAX_DUMP_KEY_BYTES`, 64 MB) are skipped and listed. Refused in cluster mode. No timeout. |
| `redis_import(events, op_id, host_id, db, input_path, replace, flush_first)` | `RedisOpSummary { processed, written, skipped_existing, skipped_big, vanished, failed, elapsed_ms }` | Validates the file as it reads. |
| `redis_cancel(&op_id)` | — | Via `ctx.cancel_op`. |
| `termdrop_core::redis::read_backup_header(&path)` | `BackupHeader { app, created_at, source, redis_version, dbs, pattern, consistency }` | Blocking: `spawn_blocking`. For the restore dialog. |

`RedisBytes { text, binary, bytes, truncated }`: `text` is UTF-8, or base64
when `binary`. Events: `CoreEvent::RedisProgress(RedisProgress { op_id, db,
stage, processed, total, percent, detail })` and
`CoreEvent::RedisCancelled { op_id, db }`.

## Rules that apply (from `CLAUDE.md`)

Both:

- The connection is named by **host id**, never by URI. The core splices
  the keyring secret in; a password never reaches the UI. Show
  `ctx.display_uri(cx)` or `RedisServerInfo::display_uri`, never a stored
  URI, and never log one (`termdrop_core::uri::redact_uri` if you must).
- Error text is the Tauri app's; `cancelled` is exact.
- Calls are cheap to repeat but not free: don't poll.

MongoDB:

- URIs go through `normalize_mongo_uri` inside the core
  (`mongodb::build_client`, `prepare_cli_uri`); never build a client or a
  CLI argument yourself.
- Dump and restore shell out to the bundled `mongodump` / `mongorestore`
  found by `resolve_mongo_tool`. `native/app/build.rs` copies
  `src-tauri/binaries/<os>-<arch>/` next to the native binary, so `cargo
  run` / `cargo test` find them; otherwise they come from `PATH`.
- `mongodb_find` is read-only by construction; keep it that way (no
  aggregate, no writes).

Redis:

- **Nothing unbounded.** Only `redis_scan_keys`, `redis_key_tree` and
  `redis_key_value`, a page at a time with "Load more"; never ask for a
  whole value or keyspace, and never trim a page the core returned (the
  cursor is only valid for the whole batch).
- **Keys are bytes.** Address a key only by its `key_b64`; `key.text` is for
  display (and is base64 when `binary` — say so in the UI).
- **One connection per `(host_id, db)`**, and the db is baked in: pass the
  db index on each call; there is no `SELECT`, and none may be issued.
- The tunnel is the host's own SSH session, opened by `redis_connect`;
  `rediss://` through a tunnel is refused with a message — show it.
- Backup files are validated before allocating (`redis_backup.rs`); show
  the error, which names an offset, rather than retrying.

### Remote desktop (`termdrop_core::vnc_ops`, types in `termdrop_core::vnc`)

| Call | Returns | Notes |
|---|---|---|
| `vnc_connect(events, host_id, password, ssh_password)` | `VncSessionInfo { session_id, width, height, desktop_name, tunnelled, security, view_only }` | Opens the host's tunnel when it has one (its own SSH session; an SSH host tunnels through itself), handshakes, starts the session thread. `password: None` reads the keyring. Wrap in `run_with_secret`. |
| `vnc_open_frame_channel(&session_id, sink)` | `()` | The `ByteSink` frames go to. No frame is requested before it, and attaching one forces a full update. |
| `vnc_frame_ack(&session_id, seq)` | `()` | After the frame is applied: the flow control. |
| `vnc_set_paused(&session_id, paused)` | `()` | While the tab or window is hidden. |
| `vnc_pointer` / `vnc_key` / `vnc_key_sequence` | `()` | Dropped by the core in view-only mode. |
| `vnc_refresh`, `vnc_set_view_only`, `vnc_send_clipboard` (`-> lossy`) | | |
| `vnc_disconnect(&session_id)` | `()` | The tab's own session (on release). The frame calls `vnc_disconnect_host` for the host's last tab. |

Events (`subscribe_session`): `VncBell`, `VncClipboard`, `VncResize`,
`VncDisconnected`, `VncError`. A missing VNC password is not a keyring
error: the server rejects the empty one, and `run_with_secret` asks ("VNC
password required") only when none is stored. A tunnel's missing SSH
password is a keyring error, asked for as for Redis. The render path is
described in `vnc/renderer.rs` and `vnc/mod.rs`.

## Lifetimes

| What | Keyed by | Built | Dropped |
|---|---|---|---|
| The tab's view | the tab (one per host and kind) | when the tab opens | when the tab closes |
| The core's pooled MongoDB client | host | first call | host's last MongoDB tab closes, or the host is edited/deleted |
| The core's Redis connections | `(host, db)` | `redis_connect` / first call on a db | host's last Redis tab closes, or the host is edited/deleted |
| The Redis tunnel | host | first `redis_connect` | as the Redis connections |
| A VNC session | the tab | `vnc_connect` | the tab closes, or the session ends |
| The VNC tunnel | host | first `vnc_connect` | the host's last session closes (`release_vnc_host`) |

A host edited while its tab is open drops the pool (`Core::update_host`), so
the next call reconnects with the new address; listen for
`AppEvent::HostsChanged` on `ctx.model` if the tab should refresh itself.

## Shared helpers

- `datastores::common`: `format_count` ("1,234"), `count_noun`,
  `format_bytes`, `missing_secret`, `OpProgress` / `OpState` +
  `op_progress_row`, `json_lines` (pretty-print, field order kept, capped) +
  `json_view` + `json_colour`, `Json` (a parsed value that keeps field order
  without serde_json's `preserve_order`, printed as `JSON.stringify` would),
  `CheckState` + `tri_check` (a none / some / all box; GPUI Kit's `Checkbox`
  has no indeterminate state), `completed_summary`, and the list styles from
  `panels::common` (`list_head`, `list_row`, `list_cell`, `row_separator`,
  `num`, `text_2xs`).
- Virtualised lists: GPUI's `uniform_list` (the SFTP panel's file list is
  the example), or `gpui_kit::component::{table, tree}`.
- `crate::ui`: `EmptyState` (empty / filtered / loading / error, kept
  distinct, each with its fix; and `notice` for a pane that waits on a
  choice made beside it, "Select a key to view it"), `PanelHeader`,
  `tag::Tag` (a header badge that is a word, toned by token, with a
  tooltip: "via bastion", "cluster"), `IconButton` (label required),
  `dialogs::{confirm, prompt}`, `theme::{palette, mono_font}`.
  Colours are palette tokens only (`p.mongo` / `p.redis` are the kinds'
  accents); filled controls use `accent_solid` / `bad_solid`; text sizes are
  the named tiers.

## Checks

From `native/`, with the linker flags in `COMPARE.md`:

```bash
cargo fmt --check
cargo clippy --offline --all-targets -- -D warnings
cargo test --offline
```

Pure logic (query parsing, key grouping labels, summaries) goes in plain
functions with `#[cfg(test)]` tests in your folder; views are not unit
tested.

Live runs: never against the real `~/.local/share/termdrop.db` or
`~/.ssh`. Set `HOME` and `XDG_DATA_HOME` to fresh scratch directories, set
`sqlite_sequence` for `hosts` to 900000 before adding hosts, and never store
secrets for or delete hosts below that id. Use the containers in `CLAUDE.md`
(`mongodb://admin:adminpass@localhost:27018`, `redis://:devpass@localhost:6380`,
the `redis-private` + `bastion` pair for the tunnel) if they are already up;
never `docker compose up` (two compose files share a `bastion` service and
clobber each other).

Run a **copy** of the binary (with `mongodump` / `mongorestore` copied
beside it), or give each agent its own `CARGO_TARGET_DIR`: two agents
sharing one target directory overwrite each other's binary mid-run. A
`tunnel@localhost:2222` bastion has to be trusted once from an SSH tab
before a tunnelled Redis host can use it, as in the Tauri app.

GTK dialogs follow GPUI onto X11 when `WAYLAND_DISPLAY` is unset
(`ui::file_dialog::align_gtk_backend`). Before that, a run with
`WAYLAND_DISPLAY` unset opened every file dialog as a WSLg Wayland window
that `xdotool` and `xwininfo` cannot see, which looked like a dead button.
