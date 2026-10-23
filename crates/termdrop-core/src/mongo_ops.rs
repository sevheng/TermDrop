//! What the MongoDB tab asks of a host: browsing (databases, collections,
//! documents, counts, indexes, stats), backup and restore through the bundled
//! `mongodump` / `mongorestore`, and releasing the host's pooled client.
//!
//! These were the bodies of the Tauri commands of the same names; error text
//! is unchanged, because the Vue frontend shows it as is and matches some of
//! it (`keyring retrieve failed`, the exact string `cancelled`).
//!
//! The connection is always named by host id. [`load_mongo_uri`] splices the
//! keyring secret into the stored URI here, so a password never reaches a UI,
//! and every client goes through `mongodb::build_client`, which applies
//! `normalize_mongo_uri`. Dump and restore report progress as
//! `CoreEvent::MongoProgress` / `MongoCancelled` on the `events` they are
//! given, keyed by the caller's `op_id`; [`Core::mongodb_cancel`] stops one.
//!
//! The password helpers (`store_mongo_secret`, `has_mongo_secret`,
//! `clear_mongo_secret`) are in `hosts.rs`, beside the other kinds'.

use std::sync::atomic::Ordering;

use crate::events::Events;
use crate::mongodb;
use crate::state::{
    forget_mongo_clients, load_mongo_uri, mongo_client, parse_optional_doc, with_async_timeout,
    with_mongo_op,
};
use crate::Core;

impl Core {
    pub async fn mongodb_list_databases(&self, host_id: i64) -> Result<Vec<String>, String> {
        let client = mongo_client(self, host_id).await?;
        with_async_timeout(mongodb::list_databases_with(&client), 30).await
    }

    pub async fn mongodb_list_collections(
        &self,
        host_id: i64,
        db: String,
    ) -> Result<Vec<String>, String> {
        let client = mongo_client(self, host_id).await?;
        with_async_timeout(mongodb::list_collections_with(&client, &db), 30).await
    }

    /// `mongodump` of `collections` (all of `db` when empty) into
    /// `output_dir`, or into one archive file when `is_archive`. Not under a
    /// timeout: a dump legitimately runs for hours; cancel it instead.
    pub async fn mongodb_dump(
        &self,
        events: Events,
        op_id: String,
        host_id: i64,
        db: String,
        collections: Vec<String>,
        output_dir: String,
        is_archive: bool,
    ) -> Result<(), String> {
        // Registered before the keyring read, so a cancel sent meanwhile is
        // not dropped as an unknown id.
        let registry_id = op_id.clone();
        with_mongo_op(self, &registry_id, |cancelled, mongo_ops| async move {
            let uri = load_mongo_uri(self, host_id)?;
            mongodb::dump_collections(
                events,
                cancelled,
                mongo_ops,
                op_id,
                &uri,
                &db,
                collections,
                &output_dir,
                is_archive,
            )
            .await
        })
        .await
    }

    /// `mongorestore` from a dump folder or archive.
    pub async fn mongodb_restore(
        &self,
        events: Events,
        op_id: String,
        host_id: i64,
        db: String,
        collections: Vec<String>,
        input_dir: String,
        is_archive: bool,
        drop_first: bool,
    ) -> Result<(), String> {
        // Registered before the keyring read, so a cancel sent meanwhile is
        // not dropped as an unknown id.
        let registry_id = op_id.clone();
        with_mongo_op(self, &registry_id, |cancelled, mongo_ops| async move {
            let uri = load_mongo_uri(self, host_id)?;
            mongodb::restore_collections(
                events,
                cancelled,
                mongo_ops,
                op_id,
                &uri,
                &db,
                collections,
                &input_dir,
                is_archive,
                drop_first,
            )
            .await
        })
        .await
    }

    /// `mongorestore --archive` limited to `includes` (`db.collection`
    /// namespaces).
    pub async fn mongodb_restore_archive(
        &self,
        events: Events,
        op_id: String,
        host_id: i64,
        includes: Vec<String>,
        input_path: String,
        drop_first: bool,
    ) -> Result<(), String> {
        // Registered before the keyring read, so a cancel sent meanwhile is
        // not dropped as an unknown id.
        let registry_id = op_id.clone();
        with_mongo_op(self, &registry_id, |cancelled, mongo_ops| async move {
            let uri = load_mongo_uri(self, host_id)?;
            mongodb::restore_archive(
                events,
                cancelled,
                mongo_ops,
                op_id,
                &uri,
                includes,
                &input_path,
                drop_first,
            )
            .await
        })
        .await
    }

    /// A page of documents from a collection.
    ///
    /// Read-only by construction: there is no aggregate command and no write
    /// command, so a pipeline stage like $out or $merge cannot reach the
    /// server from here. `filter`, `sort` and `projection` are the user's
    /// JSON text; `limit` is capped at `mongodb::MAX_FIND_LIMIT`.
    pub async fn mongodb_find(
        &self,
        host_id: i64,
        db: String,
        collection: String,
        filter: Option<String>,
        sort: Option<String>,
        projection: Option<String>,
        skip: u64,
        limit: i64,
    ) -> Result<mongodb::FindResult, String> {
        let client = mongo_client(self, host_id).await?;
        let filter = mongodb::parse_filter(filter.as_deref().unwrap_or(""))?;
        let sort = parse_optional_doc(sort.as_deref(), "sort")?;
        let projection = parse_optional_doc(projection.as_deref(), "projection")?;

        with_async_timeout(
            mongodb::find_documents(
                &client,
                &db,
                &collection,
                filter,
                sort,
                projection,
                skip,
                limit,
            ),
            60,
        )
        .await
    }

    /// Count documents matching a filter.
    pub async fn mongodb_count(
        &self,
        host_id: i64,
        db: String,
        collection: String,
        filter: Option<String>,
    ) -> Result<mongodb::CountResult, String> {
        let client = mongo_client(self, host_id).await?;
        let filter = mongodb::parse_filter(filter.as_deref().unwrap_or(""))?;
        with_async_timeout(
            mongodb::count_documents(&client, &db, &collection, filter),
            60,
        )
        .await
    }

    /// A collection's indexes.
    pub async fn mongodb_list_indexes(
        &self,
        host_id: i64,
        db: String,
        collection: String,
    ) -> Result<Vec<String>, String> {
        let client = mongo_client(self, host_id).await?;
        with_async_timeout(mongodb::list_indexes(&client, &db, &collection), 30).await
    }

    /// A collection's storage statistics.
    pub async fn mongodb_collection_stats(
        &self,
        host_id: i64,
        db: String,
        collection: String,
    ) -> Result<String, String> {
        let client = mongo_client(self, host_id).await?;
        with_async_timeout(mongodb::collection_stats(&client, &db, &collection), 30).await
    }

    /// Release the pooled client for a host: the last MongoDB tab on it
    /// closed.
    pub async fn mongodb_disconnect(&self, host_id: i64) -> Result<(), String> {
        forget_mongo_clients(self, host_id).await;
        Ok(())
    }

    /// Ask an in-flight dump or restore to stop. The operation then ends
    /// with the error `cancelled` and emits `MongoCancelled`. An unknown id
    /// (already finished) is ignored.
    pub fn mongodb_cancel(&self, op_id: &str) {
        let ops = mongodb::lock_or_recover(&self.mongo_ops);
        if let Some(handle) = ops.get(op_id) {
            handle.cancelled.store(true, Ordering::Relaxed);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::Ordering;

    use crate::state::register_mongo_op;
    use crate::state::tests_support::memory_core;

    #[test]
    fn a_missing_host_is_named_before_anything_connects() {
        let core = memory_core();
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let missing = "host 7 not found";
            assert_eq!(core.mongodb_list_databases(7).await.err().unwrap(), missing);
            assert_eq!(
                core.mongodb_find(7, "d".into(), "c".into(), None, None, None, 0, 10)
                    .await
                    .err()
                    .unwrap(),
                missing
            );
            assert_eq!(
                core.mongodb_dump(
                    std::sync::Arc::new(crate::events::NoEvents),
                    "op".into(),
                    7,
                    "d".into(),
                    vec![],
                    "/nonexistent".into(),
                    false,
                )
                .await
                .err()
                .unwrap(),
                missing
            );
            // Releasing a host that has no client is a no-op.
            assert_eq!(core.mongodb_disconnect(7).await, Ok(()));
        });
    }

    #[test]
    fn cancel_flips_the_flag_of_that_operation_only() {
        let core = memory_core();
        let a = register_mongo_op(&core, "a".into());
        let b = register_mongo_op(&core, "b".into());
        core.mongodb_cancel("a");
        core.mongodb_cancel("unknown");
        assert!(a.load(Ordering::Relaxed));
        assert!(!b.load(Ordering::Relaxed));
    }
}
