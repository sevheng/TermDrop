//! The database tree, the collections ticked for a backup, and the
//! mongodump / mongorestore runs a backup or restore turns into. Ports of
//! `src/utils/mongoSelection.js` and the job planning in
//! `useMongoBackup.js`, with its wording.

/// One database in the tree. Collections load lazily, on first expand.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DbEntry {
    pub name: String,
    pub collections: Vec<String>,
    pub loading: bool,
    /// Collections have been fetched (an empty list then means "none").
    pub loaded: bool,
}

impl DbEntry {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            collections: Vec::new(),
            loading: false,
            loaded: false,
        }
    }
}

/// A database checkbox's state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DbCheck {
    None,
    Some,
    All,
}

/// Collections ticked for the next backup, by database, in the order they
/// were ticked (the Vue `Map<db, Set<collection>>`). A database with
/// nothing ticked is not kept.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Selection {
    dbs: Vec<(String, Vec<String>)>,
}

impl Selection {
    pub fn is_selected(&self, db: &str, coll: &str) -> bool {
        self.get(db).is_some_and(|c| c.iter().any(|x| x == coll))
    }

    fn get(&self, db: &str) -> Option<&Vec<String>> {
        self.dbs.iter().find(|(d, _)| d == db).map(|(_, c)| c)
    }

    pub fn db_state(&self, db: &DbEntry) -> DbCheck {
        match self.get(&db.name) {
            None => DbCheck::None,
            Some(set) if set.is_empty() => DbCheck::None,
            Some(set) if !db.collections.is_empty() && set.len() == db.collections.len() => {
                DbCheck::All
            }
            Some(_) => DbCheck::Some,
        }
    }

    pub fn count(&self) -> usize {
        self.dbs.iter().map(|(_, c)| c.len()).sum()
    }

    pub fn is_empty(&self) -> bool {
        self.count() == 0
    }

    pub fn toggle(&mut self, db: &str, coll: &str) {
        match self.dbs.iter_mut().position(|(d, _)| d == db) {
            Some(i) => {
                let set = &mut self.dbs[i].1;
                match set.iter().position(|c| c == coll) {
                    Some(j) => {
                        set.remove(j);
                    }
                    None => set.push(coll.to_string()),
                }
                if set.is_empty() {
                    self.dbs.remove(i);
                }
            }
            None => self.dbs.push((db.to_string(), vec![coll.to_string()])),
        }
    }

    pub fn remove_db(&mut self, db: &str) {
        self.dbs.retain(|(d, _)| d != db);
    }

    /// Tick every collection of `db`, keeping its place if it had one.
    pub fn select_all(&mut self, db: &DbEntry) {
        if db.collections.is_empty() {
            self.remove_db(&db.name);
            return;
        }
        match self.dbs.iter_mut().find(|(d, _)| *d == db.name) {
            Some(slot) => slot.1 = db.collections.clone(),
            None => self.dbs.push((db.name.clone(), db.collections.clone())),
        }
    }

    pub fn clear(&mut self) {
        self.dbs.clear();
    }

    /// `[{ db, collections }]` in the order ticked (`buildEntries`).
    pub fn entries(&self) -> Vec<Entry> {
        self.dbs
            .iter()
            .map(|(db, c)| Entry {
                db: db.clone(),
                collections: c.clone(),
            })
            .collect()
    }

    /// The databases with something ticked.
    pub fn dbs(&self) -> Vec<String> {
        self.dbs.iter().map(|(d, _)| d.clone()).collect()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub db: String,
    pub collections: Vec<String>,
}

// ---- backup --------------------------------------------------------------

/// Why an archive backup cannot start. `mongodump --archive` writes one
/// file per run and `--db` is singular, so several databases cannot share
/// one archive.
pub const ARCHIVE_ONE_DB: &str = "An archive holds a single database. Select collections from one \
     database, or back up to a folder to write all of them into one tree.";

/// The archive's default file name.
pub fn archive_file_name(dbs: &[String]) -> String {
    match dbs {
        [one] => format!("{one}.gz"),
        _ => "mongodb_backup.gz".into(),
    }
}

/// The tooltip on the Backup button.
pub fn backup_tooltip(selected: usize) -> &'static str {
    if selected == 0 {
        "Select collections to back up"
    } else {
        "Back up to a folder"
    }
}

/// One mongodump or mongorestore run, with what to say after it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Job {
    pub kind: JobKind,
    pub success: String,
    pub cancelled: String,
    pub failed: String,
    /// The progress row's title.
    pub title: String,
    /// Restores only: `--drop` (set from the confirmation).
    pub drop_first: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JobKind {
    Dump {
        db: String,
        collections: Vec<String>,
        output: String,
        is_archive: bool,
    },
    Restore {
        db: String,
        collections: Vec<String>,
        input_dir: String,
    },
    RestoreArchive {
        includes: Vec<String>,
        input_path: String,
    },
}

/// One dump per selected database.
pub fn backup_jobs(entries: &[Entry], output: &str, is_archive: bool) -> Vec<Job> {
    entries
        .iter()
        .map(|e| Job {
            kind: JobKind::Dump {
                db: e.db.clone(),
                collections: e.collections.clone(),
                output: output.to_string(),
                is_archive,
            },
            success: format!("Backed up {}: {}", e.db, e.collections.join(", ")),
            cancelled: format!("Cancelled {}", e.db),
            failed: format!("Backup failed for {}", e.db),
            title: format!("Backing up {}", e.db),
            drop_first: false,
        })
        .collect()
}

/// The runs for a folder restore (`folderRestoreJobs`).
pub fn folder_restore_jobs(entries: &[Entry], source_dbs: &[String], input: &str) -> Vec<Job> {
    let whole = |db: &str, success: String| Job {
        kind: JobKind::Restore {
            db: db.to_string(),
            collections: Vec::new(),
            input_dir: input.to_string(),
        },
        success,
        cancelled: "Cancelled folder restore".into(),
        failed: "Folder restore failed".into(),
        title: if db.is_empty() {
            "Restoring folder".into()
        } else {
            format!("Restoring {db}")
        },
        drop_first: false,
    };
    match (entries.is_empty(), source_dbs) {
        // A folder holding one database: name it, or mongorestore skips the
        // files.
        (true, [one]) => vec![whole(one, format!("Restored {one}"))],
        // Nothing selected and several databases: restore everything.
        (true, _) => vec![whole("", "Restored folder".into())],
        (false, _) => entries
            .iter()
            .map(|e| Job {
                kind: JobKind::Restore {
                    db: e.db.clone(),
                    collections: e.collections.clone(),
                    input_dir: input.to_string(),
                },
                success: format!("Restored {}: {}", e.db, e.collections.join(", ")),
                cancelled: format!("Cancelled {}", e.db),
                failed: format!("Restore failed for {}", e.db),
                title: format!("Restoring {}", e.db),
                drop_first: false,
            })
            .collect(),
    }
}

/// The single run for an archive restore: one mongorestore takes every
/// selected namespace.
pub fn archive_restore_job(entries: &[Entry], input: &str) -> Job {
    let includes: Vec<String> = entries
        .iter()
        .flat_map(|e| e.collections.iter().map(move |c| format!("{}.{c}", e.db)))
        .collect();
    Job {
        kind: JobKind::RestoreArchive {
            includes,
            input_path: input.to_string(),
        },
        success: if entries.is_empty() {
            "Restored archive".into()
        } else {
            "Restored selected collections from archive".into()
        },
        cancelled: "Cancelled archive restore".into(),
        failed: "Archive restore failed".into(),
        title: "Restoring archive".into(),
        drop_first: false,
    }
}

/// `handleOperationError`'s test: a cancel stops the run quietly.
pub fn is_cancel(err: &str) -> bool {
    err.contains("cancelled")
}

// ---- the restore dialog --------------------------------------------------

/// What the restore confirmation says will be written, when no collection
/// is ticked (`MongoRestoreDialog.vue`'s fallbacks).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RestoreScope {
    /// The ticked collections, per database.
    Selected,
    /// A folder holding one database.
    OneDb(String),
    /// A folder holding several.
    AllDbs(Vec<String>),
    /// An archive, or a folder that showed nothing recognisable.
    Everything,
}

pub fn restore_scope(entries: &[Entry], is_archive: bool, source_dbs: &[String]) -> RestoreScope {
    if !entries.is_empty() {
        RestoreScope::Selected
    } else if !is_archive && source_dbs.len() == 1 {
        RestoreScope::OneDb(source_dbs[0].clone())
    } else if !is_archive && source_dbs.len() > 1 {
        RestoreScope::AllDbs(source_dbs.to_vec())
    } else {
        RestoreScope::Everything
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn db(name: &str, colls: &[&str]) -> DbEntry {
        DbEntry {
            name: name.into(),
            collections: colls.iter().map(|c| c.to_string()).collect(),
            loading: false,
            loaded: true,
        }
    }

    #[test]
    fn ticking_follows_the_vue_selection_map() {
        let mut s = Selection::default();
        let shop = db("shop", &["a", "b"]);
        assert_eq!(s.db_state(&shop), DbCheck::None);
        s.toggle("shop", "a");
        assert!(s.is_selected("shop", "a"));
        assert_eq!(s.db_state(&shop), DbCheck::Some);
        s.toggle("shop", "b");
        assert_eq!(s.db_state(&shop), DbCheck::All);
        s.toggle("logs", "x");
        assert_eq!(s.count(), 3);
        assert_eq!(s.dbs(), ["shop", "logs"]);
        s.toggle("shop", "a");
        s.toggle("shop", "b");
        // An emptied database drops out.
        assert_eq!(s.dbs(), ["logs"]);
        s.select_all(&shop);
        assert_eq!(
            s.entries(),
            [
                Entry {
                    db: "logs".into(),
                    collections: vec!["x".into()]
                },
                Entry {
                    db: "shop".into(),
                    collections: vec!["a".into(), "b".into()]
                },
            ]
        );
        s.remove_db("shop");
        assert_eq!(s.count(), 1);
        s.select_all(&db("empty", &[]));
        assert_eq!(s.dbs(), ["logs"]);
        s.clear();
        assert!(s.is_empty());
    }

    #[test]
    fn an_unloaded_database_is_never_all_ticked() {
        let mut s = Selection::default();
        s.toggle("d", "x");
        assert_eq!(s.db_state(&DbEntry::new("d")), DbCheck::Some);
    }

    #[test]
    fn backups_run_once_per_database() {
        let entries = vec![Entry {
            db: "shop".into(),
            collections: vec!["a".into(), "b".into()],
        }];
        let jobs = backup_jobs(&entries, "/tmp/out", false);
        assert_eq!(jobs.len(), 1);
        assert_eq!(jobs[0].success, "Backed up shop: a, b");
        assert_eq!(jobs[0].cancelled, "Cancelled shop");
        assert_eq!(jobs[0].failed, "Backup failed for shop");
        assert_eq!(archive_file_name(&["shop".into()]), "shop.gz");
        assert_eq!(archive_file_name(&[]), "mongodb_backup.gz");
        assert_eq!(backup_tooltip(0), "Select collections to back up");
    }

    #[test]
    fn folder_restores_name_a_lone_database() {
        let one = folder_restore_jobs(&[], &["shop".into()], "/in");
        assert_eq!(one.len(), 1);
        assert_eq!(one[0].success, "Restored shop");
        assert!(matches!(&one[0].kind, JobKind::Restore { db, .. } if db == "shop"));

        let all = folder_restore_jobs(&[], &["a".into(), "b".into()], "/in");
        assert_eq!(all[0].success, "Restored folder");
        assert!(matches!(&all[0].kind, JobKind::Restore { db, .. } if db.is_empty()));
        assert_eq!(all[0].failed, "Folder restore failed");

        let picked = folder_restore_jobs(
            &[Entry {
                db: "x".into(),
                collections: vec!["c".into()],
            }],
            &["a".into()],
            "/in",
        );
        assert_eq!(picked[0].success, "Restored x: c");
        assert_eq!(picked[0].failed, "Restore failed for x");
    }

    #[test]
    fn archive_restores_include_namespaces() {
        let job = archive_restore_job(
            &[Entry {
                db: "d".into(),
                collections: vec!["a".into(), "b".into()],
            }],
            "/f.gz",
        );
        assert!(matches!(&job.kind, JobKind::RestoreArchive { includes, .. }
            if includes == &["d.a".to_string(), "d.b".to_string()]));
        assert_eq!(job.success, "Restored selected collections from archive");
        assert_eq!(archive_restore_job(&[], "/f").success, "Restored archive");
    }

    #[test]
    fn the_dialog_says_what_will_be_restored() {
        let e = vec![Entry {
            db: "d".into(),
            collections: vec!["a".into()],
        }];
        assert_eq!(restore_scope(&e, true, &[]), RestoreScope::Selected);
        assert_eq!(
            restore_scope(&[], false, &["x".into()]),
            RestoreScope::OneDb("x".into())
        );
        assert_eq!(
            restore_scope(&[], false, &["x".into(), "y".into()]),
            RestoreScope::AllDbs(vec!["x".into(), "y".into()])
        );
        assert_eq!(
            restore_scope(&[], true, &["x".into()]),
            RestoreScope::Everything
        );
        assert!(is_cancel("cancelled"));
        assert!(!is_cancel("connection refused"));
    }
}
