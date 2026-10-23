//! What the Redis tab asks of a host: connecting (through its SSH tunnel when
//! it has one), the server's INFO, bounded key scans, the prefix tree, one
//! key's value a page at a time, backup to and restore from a `.tdredis`
//! file, and releasing the host's pooled connections and tunnel.
//!
//! These were the bodies of the Tauri commands of the same names; error text
//! is unchanged, because the Vue frontend shows it as is and matches some of
//! it (`keyring retrieve failed`).
//!
//! The rules in `redis.rs` hold for every call here: reads are `SCAN`
//! variants or explicit ranges, capped; connections are pooled per
//! `(host_id, db_index)` with the index baked in, so `SELECT` is never
//! issued; a key is addressed by its exact bytes as `key_b64`; the tunnel is
//! its own SSH session (`redis_tunnel_port`); `rediss://` through a tunnel is
//! refused (`redis_dial_uri`). A URI handed to a UI is always `redact_uri`d.
//!
//! Backup and restore report progress as `CoreEvent::RedisProgress` /
//! `RedisCancelled` on the `events` they are given, keyed by the caller's
//! `op_id`; [`Core::redis_cancel`] stops one. They are deliberately not under
//! a timeout: a large keyspace legitimately takes a long time.
//!
//! The password helpers (`store_redis_secret`, `has_redis_secret`,
//! `clear_redis_secret`) are in `hosts.rs`; reading a backup's header is
//! `redis::read_backup_header`, which needs no connection.

use std::sync::atomic::Ordering;
use std::time::Duration;

use crate::events::Events;
use crate::state::{
    forget_redis, redis_conn, redis_dial_uri, with_async_timeout, with_db, with_redis_op,
};
use crate::{db, redis, uri, Core};

impl Core {
    fn redis_host(&self, host_id: i64) -> Result<db::Host, String> {
        with_db(self, |conn| db::get_host_by_id(conn, host_id))?
            .ok_or_else(|| format!("host {} not found", host_id))
    }

    /// Open a Redis connection and report what the server is.
    ///
    /// `ssh_password` supplies the bastion's password on the prompt-and-retry
    /// path, for a tunnelled host whose SSH password is not in the keyring.
    pub async fn redis_connect(
        &self,
        host_id: i64,
        ssh_password: Option<String>,
    ) -> Result<redis::RedisServerInfo, String> {
        let host = self.redis_host(host_id)?;

        let (uri, tunnelled) = redis_dial_uri(self, &host, ssh_password).await?;
        let db_index = redis::uri_db_index(&uri).unwrap_or(0);

        let mut conn = redis::build_client(&uri, db_index).await?;
        let info = with_async_timeout(
            redis::server_info(&mut conn, uri::redact_uri(&uri), tunnelled),
            30,
        )
        .await?;

        self.redis_conns
            .lock()
            .await
            .insert((host_id, db_index), conn);
        Ok(info)
    }

    /// Re-read the server's INFO, so key counts follow a backup or restore.
    pub async fn redis_server_info(&self, host_id: i64) -> Result<redis::RedisServerInfo, String> {
        let host = self.redis_host(host_id)?;
        let (uri, tunnelled) = redis_dial_uri(self, &host, None).await?;
        let db_index = redis::uri_db_index(&uri).unwrap_or(0);
        let mut conn = redis_conn(self, host_id, db_index).await?;
        with_async_timeout(
            redis::server_info(&mut conn, uri::redact_uri(&uri), tunnelled),
            30,
        )
        .await
    }

    /// Drop this host's pooled connections and close its tunnel: the last
    /// Redis tab on it closed.
    pub async fn redis_disconnect(&self, host_id: i64) -> Result<(), String> {
        forget_redis(self, host_id).await;
        Ok(())
    }

    /// One page of a database's keyspace.
    ///
    /// `cursor` is a decimal string (Redis cursors are u64); `"0"` both
    /// starts and ends an iteration. An empty `pattern` or `type_filter` is
    /// none. The page is never trimmed to `limit`: the cursor is only valid
    /// for the whole batch.
    pub async fn redis_scan_keys(
        &self,
        host_id: i64,
        db: i64,
        cursor: String,
        pattern: Option<String>,
        type_filter: Option<String>,
        limit: i64,
        with_memory: bool,
        supports_scan_type: bool,
    ) -> Result<redis::RedisScanPage, String> {
        let mut conn = redis_conn(self, host_id, db).await?;
        with_async_timeout(
            redis::scan_keys(
                &mut conn,
                &cursor,
                pattern.as_deref().filter(|p| !p.is_empty()),
                type_filter.as_deref().filter(|t| !t.is_empty()),
                limit,
                with_memory,
                supports_scan_type,
            ),
            30,
        )
        .await
    }

    /// The prefix groups of a database, from a bounded scan
    /// (`redis::TREE_SCAN_CAP` keys or 3 seconds).
    pub async fn redis_key_tree(
        &self,
        host_id: i64,
        db: i64,
        pattern: Option<String>,
    ) -> Result<redis::KeyTree, String> {
        let mut conn = redis_conn(self, host_id, db).await?;
        with_async_timeout(
            redis::key_tree(
                &mut conn,
                pattern.as_deref().filter(|p| !p.is_empty()),
                redis::TREE_SCAN_CAP,
                Duration::from_secs(3),
            ),
            30,
        )
        .await
    }

    /// One page of a single key's value.
    ///
    /// `key_b64` carries the key's exact bytes: a key that is not valid UTF-8
    /// has no faithful string form, and addressing it by a lossy one would
    /// fetch the wrong key or nothing at all. `cursor` drives hashes and
    /// sets (`HSCAN`/`SSCAN`, `None` is `"0"`), `offset` lists and sorted
    /// sets.
    pub async fn redis_key_value(
        &self,
        host_id: i64,
        db: i64,
        key_b64: String,
        cursor: Option<String>,
        offset: i64,
        limit: i64,
    ) -> Result<redis::RedisValuePage, String> {
        let key = redis::decode_key(&key_b64)?;
        let mut conn = redis_conn(self, host_id, db).await?;
        with_async_timeout(
            redis::key_value(
                &mut conn,
                &key,
                cursor.as_deref().unwrap_or("0"),
                offset,
                limit,
            ),
            60,
        )
        .await
    }

    /// Ask an in-flight export or import to stop. An unknown id (already
    /// finished) is ignored.
    pub fn redis_cancel(&self, op_id: &str) {
        let ops = self.redis_ops.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(flag) = ops.get(op_id) {
            flag.store(true, Ordering::Relaxed);
        }
    }

    /// Back up a database to a `.tdredis` file. `app_version` goes into the
    /// file's header (`TermDrop <app_version>`); the Tauri app passes its
    /// crate version. `max_key_bytes` defaults to
    /// `redis::MAX_DUMP_KEY_BYTES`; larger keys are skipped and listed.
    pub async fn redis_export(
        &self,
        events: Events,
        op_id: String,
        host_id: i64,
        db: i64,
        pattern: Option<String>,
        output_path: String,
        max_key_bytes: Option<u64>,
        app_version: &str,
    ) -> Result<redis::RedisOpSummary, String> {
        let host = self.redis_host(host_id)?;
        let (uri, tunnelled) = redis_dial_uri(self, &host, None).await?;

        let mut conn = redis_conn(self, host_id, db).await?;
        let info = redis::server_info(&mut conn, uri::redact_uri(&uri), tunnelled).await?;

        let registry_id = op_id.clone();
        with_redis_op(self, &registry_id, |cancelled| async move {
            redis::export_keys(
                &events,
                cancelled,
                &op_id,
                &mut conn,
                db,
                pattern.as_deref().filter(|p| !p.is_empty()),
                &output_path,
                max_key_bytes.unwrap_or(redis::MAX_DUMP_KEY_BYTES),
                &uri,
                &info.version,
                &info.mode,
                app_version,
            )
            .await
        })
        .await
    }

    /// Restore a `.tdredis` file into a database. The file is validated as it
    /// is read (every length capped before allocating, record count and
    /// CRC32 checked), so a truncated or hostile file errors with an offset.
    pub async fn redis_import(
        &self,
        events: Events,
        op_id: String,
        host_id: i64,
        db: i64,
        input_path: String,
        replace: bool,
        flush_first: bool,
    ) -> Result<redis::RedisOpSummary, String> {
        let host = self.redis_host(host_id)?;
        let (uri, tunnelled) = redis_dial_uri(self, &host, None).await?;

        let mut conn = redis_conn(self, host_id, db).await?;
        let info = redis::server_info(&mut conn, uri::redact_uri(&uri), tunnelled).await?;

        let registry_id = op_id.clone();
        with_redis_op(self, &registry_id, |cancelled| async move {
            redis::import_keys(
                &events,
                cancelled,
                &op_id,
                &mut conn,
                db,
                &input_path,
                replace,
                flush_first,
                &info.version,
            )
            .await
        })
        .await
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;

    use crate::events::NoEvents;
    use crate::state::tests_support::memory_core;

    #[test]
    fn a_missing_host_is_named_before_anything_connects() {
        let core = memory_core();
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let missing = "host 7 not found";
            assert_eq!(core.redis_connect(7, None).await.err().unwrap(), missing);
            assert_eq!(core.redis_server_info(7).await.err().unwrap(), missing);
            assert_eq!(
                core.redis_scan_keys(7, 0, "0".into(), None, None, 50, false, true)
                    .await
                    .err()
                    .unwrap(),
                missing
            );
            assert_eq!(
                core.redis_export(
                    Arc::new(NoEvents),
                    "op".into(),
                    7,
                    0,
                    None,
                    "/nonexistent".into(),
                    None,
                    "0.0.0",
                )
                .await
                .err()
                .unwrap(),
                missing
            );
            assert_eq!(core.redis_disconnect(7).await, Ok(()));
        });
    }

    #[test]
    fn a_key_that_is_not_base64_is_refused_before_connecting() {
        let core = memory_core();
        let rt = tokio::runtime::Runtime::new().unwrap();
        let err = rt
            .block_on(core.redis_key_value(7, 0, "not base64!".into(), None, 0, 10))
            .err()
            .unwrap();
        assert!(!err.contains("not found"), "{err}");
    }

    #[test]
    fn cancel_flips_the_flag_of_that_operation_only() {
        let core = memory_core();
        let a = Arc::new(AtomicBool::new(false));
        let b = Arc::new(AtomicBool::new(false));
        {
            let mut ops = core.redis_ops.lock().unwrap();
            ops.insert("a".into(), a.clone());
            ops.insert("b".into(), b.clone());
        }
        core.redis_cancel("a");
        core.redis_cancel("unknown");
        assert!(a.load(Ordering::Relaxed));
        assert!(!b.load(Ordering::Relaxed));
    }
}
