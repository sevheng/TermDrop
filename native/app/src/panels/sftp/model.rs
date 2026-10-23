//! The SFTP panel's rules, as plain functions: sorting, filtering, the
//! breadcrumb, selection, the column preference, the paths each action
//! builds, the preview's file type and the messages bulk actions end with.
//! Ports of the logic in `SftpPanel.vue` and `FilePreviewDialog.vue`; the
//! view in `mod.rs` only lays these out.

use std::cmp::Ordering;

use serde_json::{Map, Value};
use termdrop_core::sftp::SftpFile;

use crate::panels::common::{join_remote, parent_dir};

/// The setting the column choice is stored under, shared with the Tauri app
/// (`usePrefs.js`'s `sftp-columns`, JSON).
pub const COLUMNS_KEY: &str = "sftp-columns";

// ---- sorting and filtering -----------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SortKey {
    #[default]
    Name,
    Size,
    Modified,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SortOrder {
    #[default]
    Asc,
    Desc,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Sort {
    pub key: SortKey,
    pub order: SortOrder,
}

impl Sort {
    /// A header click: the same column flips the order, another one sorts by
    /// it ascending. `setSort`.
    pub fn toggle(self, key: SortKey) -> Self {
        if self.key == key {
            let order = match self.order {
                SortOrder::Asc => SortOrder::Desc,
                SortOrder::Desc => SortOrder::Asc,
            };
            Self { key, order }
        } else {
            Self {
                key,
                order: SortOrder::Asc,
            }
        }
    }

    /// The arrow after a header label, empty for the other columns.
    /// `sortIndicator`.
    pub fn indicator(self, key: SortKey) -> &'static str {
        if self.key != key {
            ""
        } else if self.order == SortOrder::Asc {
            "\u{25B2}"
        } else {
            "\u{25BC}"
        }
    }
}

/// `String.localeCompare` as an English system orders file names: case is
/// ignored first, and on a tie the lower-case name comes first.
pub fn locale_cmp(a: &str, b: &str) -> Ordering {
    a.to_lowercase()
        .cmp(&b.to_lowercase())
        .then_with(|| b.cmp(a))
}

/// Directories first, always; then by the sort key, in its order.
/// `sortedFiles`.
pub fn sort_files(files: &mut [SftpFile], sort: Sort) {
    files.sort_by(|a, b| {
        if a.is_dir != b.is_dir {
            return if a.is_dir {
                Ordering::Less
            } else {
                Ordering::Greater
            };
        }
        let cmp = match sort.key {
            SortKey::Name => locale_cmp(&a.name, &b.name),
            SortKey::Size => a.size.cmp(&b.size),
            SortKey::Modified => a.modified.unwrap_or(0).cmp(&b.modified.unwrap_or(0)),
        };
        match sort.order {
            SortOrder::Asc => cmp,
            SortOrder::Desc => cmp.reverse(),
        }
    });
}

/// The indexes of the files whose name contains `query`, ignoring case and
/// surrounding blanks. `filteredFiles`.
pub fn filter_indexes(files: &[SftpFile], query: &str) -> Vec<usize> {
    let q = query.trim().to_lowercase();
    files
        .iter()
        .enumerate()
        .filter(|(_, f)| q.is_empty() || f.name.to_lowercase().contains(&q))
        .map(|(i, _)| i)
        .collect()
}

// ---- paths ---------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Crumb {
    pub name: String,
    pub path: String,
}

/// The path bar: "root" for `/`, then one entry per component. `breadcrumbs`.
pub fn breadcrumbs(path: &str) -> Vec<Crumb> {
    let normalized = path.replace('\\', "/");
    let mut out = vec![Crumb {
        name: "root".into(),
        path: "/".into(),
    }];
    let mut acc = String::new();
    for part in normalized.split('/').filter(|p| !p.is_empty()) {
        acc.push('/');
        acc.push_str(part);
        out.push(Crumb {
            name: part.into(),
            path: acc.clone(),
        });
    }
    out
}

/// The directory "Go up" moves to; `None` at the root. `goUp`.
pub fn up_dir(path: &str) -> Option<String> {
    if path == "/" {
        return None;
    }
    let parts: Vec<&str> = path.split('/').filter(|p| !p.is_empty()).collect();
    if parts.len() <= 1 {
        Some("/".into())
    } else {
        Some(format!("/{}", parts[..parts.len() - 1].join("/")))
    }
}

/// Where a new entry called `name` goes in `dir`. The Vue panel built a
/// relative path at `/` (it dropped the slash), which put an upload to the
/// root in the home directory instead; this keeps it absolute.
pub fn child_path(dir: &str, name: &str) -> String {
    join_remote(dir, name)
}

/// The path `file` takes when renamed to `new_name`, in the same directory.
pub fn renamed_path(path: &str, new_name: &str) -> String {
    join_remote(&parent_dir(path), new_name)
}

/// The last component of a local path, for the remote name of an upload.
/// Both separators count, as `split(/[\\/]/)` did.
pub fn local_file_name(local: &str) -> Option<&str> {
    local
        .rsplit(['/', '\\'])
        .next()
        .filter(|name| !name.is_empty())
}

// ---- selection -----------------------------------------------------------

/// How a row was clicked.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ClickMods {
    /// Ctrl (Cmd on macOS), or the row's checkbox: toggle this row.
    pub toggle: bool,
    /// Shift: extend from the last clicked row.
    pub range: bool,
}

/// The selected rows, by remote path, in the order they were selected (the
/// order bulk actions run in, as the Vue `Set` iterated).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Selection {
    paths: Vec<String>,
    last_index: Option<usize>,
}

impl Selection {
    pub fn contains(&self, path: &str) -> bool {
        self.paths.iter().any(|p| p == path)
    }

    pub fn len(&self) -> usize {
        self.paths.len()
    }

    pub fn is_empty(&self) -> bool {
        self.paths.is_empty()
    }

    pub fn paths(&self) -> &[String] {
        &self.paths
    }

    pub fn clear(&mut self) {
        self.paths.clear();
        self.last_index = None;
    }

    fn add(&mut self, path: &str) {
        if !self.contains(path) {
            self.paths.push(path.to_string());
        }
    }

    /// A click on row `index` of `visible` (the filtered, sorted paths).
    /// `handleFileClick`.
    pub fn click(&mut self, visible: &[&str], index: usize, mods: ClickMods) {
        let Some(path) = visible.get(index).copied() else {
            return;
        };
        if mods.toggle {
            if self.contains(path) {
                self.paths.retain(|p| p != path);
            } else {
                self.add(path);
            }
            self.last_index = Some(index);
        } else if mods.range
            && let Some(last) = self.last_index
        {
            let (start, end) = (last.min(index), last.max(index));
            for p in visible.iter().take(end + 1).skip(start) {
                self.add(p);
            }
        } else {
            self.clear();
            self.add(path);
            self.last_index = Some(index);
        }
    }

    /// A right-click on `path`: whether the menu acts on the whole selection.
    /// A row outside a multiple selection becomes the only selected one.
    /// `showContextMenu`.
    pub fn context(&mut self, path: &str) -> bool {
        let multi = self.len() > 1 && self.contains(path);
        if !multi && !self.is_empty() {
            self.clear();
            self.add(path);
        }
        multi
    }
}

// ---- columns -------------------------------------------------------------

/// Which optional columns show. All off unless the setting says otherwise.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Columns {
    pub size: bool,
    pub modified: bool,
    pub perms: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Column {
    Size,
    Modified,
    Perms,
}

impl Column {
    pub const ALL: [Column; 3] = [Column::Size, Column::Modified, Column::Perms];

    pub fn label(self) -> &'static str {
        match self {
            Column::Size => "Size",
            Column::Modified => "Modified",
            Column::Perms => "Perms",
        }
    }

    fn key(self) -> &'static str {
        match self {
            Column::Size => "size",
            Column::Modified => "modified",
            Column::Perms => "perms",
        }
    }
}

impl Columns {
    pub fn get(self, column: Column) -> bool {
        match column {
            Column::Size => self.size,
            Column::Modified => self.modified,
            Column::Perms => self.perms,
        }
    }

    pub fn set(&mut self, column: Column, on: bool) {
        match column {
            Column::Size => self.size = on,
            Column::Modified => self.modified = on,
            Column::Perms => self.perms = on,
        }
    }
}

/// The stored preference: the three columns plus whatever else the object
/// held, which is written back untouched (the Vue panel spread the saved
/// value over its defaults).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ColumnsPref {
    pub columns: Columns,
    rest: Map<String, Value>,
}

impl ColumnsPref {
    /// Decode the setting. Anything that is not a JSON object is ignored in
    /// favour of the defaults, as `usePrefs.js`'s `decode` does; a column
    /// that is not a boolean reads by JavaScript truthiness.
    pub fn parse(raw: Option<&str>) -> Self {
        let Some(Value::Object(mut map)) = raw.and_then(|r| serde_json::from_str(r).ok()) else {
            return Self::default();
        };
        let mut columns = Columns::default();
        for column in Column::ALL {
            if let Some(v) = map.remove(column.key()) {
                columns.set(column, truthy(&v));
            }
        }
        Self { columns, rest: map }
    }

    pub fn to_json(&self) -> String {
        let mut map = self.rest.clone();
        for column in Column::ALL {
            map.insert(column.key().into(), Value::Bool(self.columns.get(column)));
        }
        Value::Object(map).to_string()
    }
}

fn truthy(v: &Value) -> bool {
    match v {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_f64().is_some_and(|f| f != 0.0 && !f.is_nan()),
        Value::String(s) => !s.is_empty(),
        Value::Array(_) | Value::Object(_) => true,
    }
}

// ---- preview -------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PreviewKind {
    Text,
    /// Shown as a picture, in this format.
    Image(ImageKind),
    /// Not previewed: offers a download instead.
    Binary,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageKind {
    Png,
    Jpeg,
    Gif,
    Svg,
    Webp,
    Bmp,
    Ico,
}

/// By extension, as `detectFileType`: the text after the last dot, or the
/// whole name when it has none.
pub fn preview_kind(file_name: &str) -> PreviewKind {
    let ext = file_name
        .rsplit('.')
        .next()
        .unwrap_or_default()
        .to_lowercase();
    let image = match ext.as_str() {
        "png" => Some(ImageKind::Png),
        "jpg" | "jpeg" => Some(ImageKind::Jpeg),
        "gif" => Some(ImageKind::Gif),
        "svg" => Some(ImageKind::Svg),
        "webp" => Some(ImageKind::Webp),
        "bmp" => Some(ImageKind::Bmp),
        "ico" => Some(ImageKind::Ico),
        _ => None,
    };
    if let Some(kind) = image {
        return PreviewKind::Image(kind);
    }
    const BINARIES: [&str; 22] = [
        "zip", "tar", "gz", "bz2", "xz", "deb", "rpm", "exe", "dll", "so", "dylib", "bin", "dat",
        "pdf", "doc", "docx", "xls", "xlsx", "ppt", "pptx", "7z", "rar",
    ];
    if BINARIES.contains(&ext.as_str()) {
        PreviewKind::Binary
    } else {
        PreviewKind::Text
    }
}

/// The directory to go back to if a navigation's listing fails.
///
/// It lives on the panel rather than in the listing's callback because a
/// reload (after a delete, an upload, ...) replaces the listing task in
/// flight, and with it any callback: the path bar would then stay on a
/// directory that could not be listed. Vue's `navigateTo` awaited its own
/// `loadFiles` and always restored. Whichever listing settles next settles
/// it, since every listing reads the current path.
#[derive(Debug, Default)]
pub struct PendingRestore(Option<String>);

impl PendingRestore {
    /// A navigation away from `previous` started. While an earlier one is
    /// still unconfirmed its directory is the last one that listed, so it
    /// is kept.
    pub fn navigating(&mut self, previous: String) {
        self.0.get_or_insert(previous);
    }

    /// A listing finished: the directory to go back to, if it failed while a
    /// navigation was pending. A failed reload with nothing pending stays put.
    pub fn settle(&mut self, ok: bool) -> Option<String> {
        let pending = self.0.take();
        if ok { None } else { pending }
    }
}

/// A preview search's counter: "2/5", or "0/0" when nothing matched.
pub fn match_label(current: usize, count: usize) -> String {
    if count == 0 {
        "0/0".into()
    } else {
        format!("{}/{}", current.min(count - 1) + 1, count)
    }
}

// ---- messages ------------------------------------------------------------

/// `"s"` when `n > 1`, as the Vue messages pluralise (so 0 reads singular).
fn plural(n: usize) -> &'static str {
    if n > 1 { "s" } else { "" }
}

/// The delete confirmation for one entry: (title, message).
pub fn delete_prompt(name: &str, is_dir: bool) -> (&'static str, String) {
    if is_dir {
        (
            "Delete Folder",
            format!("Delete \"{name}\" and all its contents? This cannot be undone."),
        )
    } else {
        (
            "Delete File",
            format!("Delete \"{name}\"? This cannot be undone."),
        )
    }
}

pub fn bulk_delete_prompt(count: usize) -> String {
    format!(
        "Delete {count} selected item{}? This cannot be undone.",
        plural(count)
    )
}

/// How a bulk action ended: the toast and whether it is a warning.
pub fn bulk_delete_result(deleted: usize, failed: usize) -> (String, bool) {
    if failed > 0 {
        (format!("Deleted {deleted}, failed {failed}"), true)
    } else {
        (format!("Deleted {deleted} item{}", plural(deleted)), false)
    }
}

pub fn bulk_download_result(completed: usize, failed: usize) -> (String, bool) {
    if failed > 0 {
        (format!("Downloaded {completed}, failed {failed}"), true)
    } else {
        (
            format!("Downloaded {completed} file{}", plural(completed)),
            false,
        )
    }
}

/// The empty list's title and hint: (empty directory, or so many hidden by
/// the filter).
pub fn empty_hint(total: usize) -> Option<String> {
    (total > 0).then(|| format!("{total} hidden by the filter"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_failed_navigation_goes_back_even_after_a_reload_replaced_it() {
        // navigate /home -> /root; a reload replaces that listing and fails.
        let mut r = PendingRestore::default();
        r.navigating("/home".into());
        assert_eq!(r.settle(false).as_deref(), Some("/home"));
        // Nothing pending: a failed reload stays where it is.
        assert_eq!(r.settle(false), None);
        // A success confirms the new directory.
        r.navigating("/home".into());
        assert_eq!(r.settle(true), None);
        assert_eq!(r.settle(false), None);
        // Two navigations before either settles go back to the first's origin.
        r.navigating("/home".into());
        r.navigating("/root".into());
        assert_eq!(r.settle(false).as_deref(), Some("/home"));
    }

    fn file(name: &str, is_dir: bool, size: u64, modified: Option<u64>) -> SftpFile {
        SftpFile {
            name: name.into(),
            path: format!("/d/{name}"),
            size,
            is_dir,
            modified,
            permissions: None,
            uid: None,
            gid: None,
        }
    }

    fn names(files: &[SftpFile]) -> Vec<&str> {
        files.iter().map(|f| f.name.as_str()).collect()
    }

    #[test]
    fn directories_come_first_in_either_order() {
        let mut files = vec![
            file("b.txt", false, 10, Some(3)),
            file("Zeta", true, 0, Some(1)),
            file("a.txt", false, 30, Some(1)),
            file("alpha", true, 0, Some(2)),
        ];
        sort_files(&mut files, Sort::default());
        assert_eq!(names(&files), ["alpha", "Zeta", "a.txt", "b.txt"]);
        sort_files(&mut files, Sort::default().toggle(SortKey::Name));
        assert_eq!(names(&files), ["Zeta", "alpha", "b.txt", "a.txt"]);
        let by_size = Sort {
            key: SortKey::Size,
            order: SortOrder::Asc,
        };
        sort_files(&mut files, by_size);
        assert_eq!(names(&files)[2..], ["b.txt", "a.txt"]);
        let by_date = Sort {
            key: SortKey::Modified,
            order: SortOrder::Desc,
        };
        sort_files(&mut files, by_date);
        assert_eq!(names(&files), ["alpha", "Zeta", "b.txt", "a.txt"]);
    }

    #[test]
    fn names_compare_like_locale_compare() {
        assert_eq!(locale_cmp("apple", "Banana"), Ordering::Less);
        assert_eq!(locale_cmp("a", "A"), Ordering::Less);
        assert_eq!(locale_cmp("A", "a"), Ordering::Greater);
        assert_eq!(locale_cmp("x", "x"), Ordering::Equal);
    }

    #[test]
    fn sort_toggles_and_indicates() {
        let s = Sort::default();
        assert_eq!(s.indicator(SortKey::Name), "\u{25B2}");
        assert_eq!(s.indicator(SortKey::Size), "");
        let s = s.toggle(SortKey::Name);
        assert_eq!(s.order, SortOrder::Desc);
        assert_eq!(s.indicator(SortKey::Name), "\u{25BC}");
        let s = s.toggle(SortKey::Size);
        assert_eq!(
            s,
            Sort {
                key: SortKey::Size,
                order: SortOrder::Asc
            }
        );
    }

    #[test]
    fn filter_ignores_case_and_blanks() {
        let files = vec![
            file("Readme.MD", false, 0, None),
            file("src", true, 0, None),
        ];
        assert_eq!(filter_indexes(&files, ""), [0, 1]);
        assert_eq!(filter_indexes(&files, "  readme "), [0]);
        assert!(filter_indexes(&files, "zzz").is_empty());
    }

    #[test]
    fn breadcrumbs_start_at_root() {
        let crumbs = breadcrumbs("/home//me/");
        let pairs: Vec<_> = crumbs
            .iter()
            .map(|c| (c.name.as_str(), c.path.as_str()))
            .collect();
        assert_eq!(
            pairs,
            [("root", "/"), ("home", "/home"), ("me", "/home/me")]
        );
        assert_eq!(breadcrumbs("/").len(), 1);
    }

    #[test]
    fn up_stops_at_root() {
        assert_eq!(up_dir("/"), None);
        assert_eq!(up_dir("/etc").as_deref(), Some("/"));
        assert_eq!(up_dir("/var/log").as_deref(), Some("/var"));
    }

    #[test]
    fn new_paths_stay_absolute() {
        assert_eq!(child_path("/", "a.txt"), "/a.txt");
        assert_eq!(child_path("/home/me", "new dir"), "/home/me/new dir");
        assert_eq!(renamed_path("/home/me/a", "b"), "/home/me/b");
        assert_eq!(renamed_path("/a", "b"), "/b");
        assert_eq!(local_file_name("/tmp/x/report.pdf"), Some("report.pdf"));
        assert_eq!(local_file_name("C:\\Users\\me\\a.txt"), Some("a.txt"));
        assert_eq!(local_file_name("/tmp/dir/"), None);
    }

    #[test]
    fn clicks_select_toggle_and_extend() {
        let rows = ["/a", "/b", "/c", "/d"];
        let mut s = Selection::default();
        s.click(&rows, 1, ClickMods::default());
        assert_eq!(s.paths(), ["/b"]);
        s.click(
            &rows,
            3,
            ClickMods {
                range: true,
                ..Default::default()
            },
        );
        assert_eq!(s.paths(), ["/b", "/c", "/d"]);
        s.click(
            &rows,
            2,
            ClickMods {
                toggle: true,
                ..Default::default()
            },
        );
        assert_eq!(s.paths(), ["/b", "/d"]);
        s.click(&rows, 0, ClickMods::default());
        assert_eq!(s.paths(), ["/a"]);
        // Shift with no anchor is a plain click.
        let mut fresh = Selection::default();
        fresh.click(
            &rows,
            2,
            ClickMods {
                range: true,
                ..Default::default()
            },
        );
        assert_eq!(fresh.paths(), ["/c"]);
    }

    #[test]
    fn right_click_keeps_a_multiple_selection_it_lands_in() {
        let rows = ["/a", "/b", "/c"];
        let mut s = Selection::default();
        s.click(&rows, 0, ClickMods::default());
        s.click(
            &rows,
            1,
            ClickMods {
                toggle: true,
                ..Default::default()
            },
        );
        assert!(s.context("/b"));
        assert_eq!(s.len(), 2);
        assert!(!s.context("/c"));
        assert_eq!(s.paths(), ["/c"]);
        // Nothing selected: the menu is for the row, and selects nothing.
        let mut empty = Selection::default();
        assert!(!empty.context("/a"));
        assert!(empty.is_empty());
    }

    #[test]
    fn columns_default_off_and_keep_unknown_keys() {
        assert_eq!(ColumnsPref::parse(None).columns, Columns::default());
        assert_eq!(ColumnsPref::parse(Some("nope")).columns, Columns::default());
        assert_eq!(ColumnsPref::parse(Some("[1]")).columns, Columns::default());
        let mut pref = ColumnsPref::parse(Some(r#"{"size":true,"perms":1,"extra":"x"}"#));
        assert_eq!(
            pref.columns,
            Columns {
                size: true,
                modified: false,
                perms: true
            }
        );
        pref.columns.set(Column::Modified, true);
        let v: Value = serde_json::from_str(&pref.to_json()).unwrap();
        assert_eq!(
            v,
            serde_json::json!({"size": true, "modified": true, "perms": true, "extra": "x"})
        );
    }

    #[test]
    fn preview_kind_follows_the_extension() {
        assert_eq!(preview_kind("a.PNG"), PreviewKind::Image(ImageKind::Png));
        assert_eq!(preview_kind("a.jpg"), PreviewKind::Image(ImageKind::Jpeg));
        assert_eq!(preview_kind("x.svg"), PreviewKind::Image(ImageKind::Svg));
        assert_eq!(preview_kind("a.tar.gz"), PreviewKind::Binary);
        assert_eq!(preview_kind("zip"), PreviewKind::Binary);
        assert_eq!(preview_kind("notes.txt"), PreviewKind::Text);
        assert_eq!(preview_kind("Makefile"), PreviewKind::Text);
        assert_eq!(preview_kind("odd."), PreviewKind::Text);
        assert_eq!(match_label(0, 0), "0/0");
        assert_eq!(match_label(1, 5), "2/5");
        assert_eq!(match_label(9, 3), "3/3");
    }

    #[test]
    fn messages_match_the_vue_wording() {
        assert_eq!(
            delete_prompt("x", true),
            (
                "Delete Folder",
                "Delete \"x\" and all its contents? This cannot be undone.".into()
            )
        );
        assert_eq!(
            delete_prompt("x", false).1,
            "Delete \"x\"? This cannot be undone."
        );
        assert_eq!(
            bulk_delete_prompt(1),
            "Delete 1 selected item? This cannot be undone."
        );
        assert_eq!(
            bulk_delete_prompt(3),
            "Delete 3 selected items? This cannot be undone."
        );
        assert_eq!(bulk_delete_result(2, 0), ("Deleted 2 items".into(), false));
        assert_eq!(
            bulk_delete_result(1, 1),
            ("Deleted 1, failed 1".into(), true)
        );
        assert_eq!(
            bulk_download_result(1, 0),
            ("Downloaded 1 file".into(), false)
        );
        assert_eq!(
            bulk_download_result(0, 2),
            ("Downloaded 0, failed 2".into(), true)
        );
        assert_eq!(empty_hint(0), None);
        assert_eq!(empty_hint(4).as_deref(), Some("4 hidden by the filter"));
    }
}
