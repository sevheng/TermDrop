//! Arrow-key navigation over a list that may contain headers. Port of
//! `src/utils/listNav.js`.
//!
//! Generic on purpose: the host sidebar is a tree of collapsible groups, but
//! the same walk serves any list with non-selectable rows. Indices are
//! `isize` with `-1` for "nothing", exactly as in the JS, so the ported cases
//! read the same.

use std::collections::HashSet;

/// The pixel height of one host row, shared so a virtual list and the row
/// style cannot disagree.
pub const HOST_ROW_H: f32 = 32.0;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Row<T> {
    Group { name: String },
    Item { item: T, group: String },
}

impl<T> Row<T> {
    pub fn is_item(&self) -> bool {
        matches!(self, Row::Item { .. })
    }
}

/// Flatten groups into the sequence the eye actually sees. Rows inside a
/// collapsed group are absent, not merely hidden: arrowing must skip them.
pub fn visible_rows<'a, T: Copy + 'a>(
    groups: impl IntoIterator<Item = (&'a str, &'a [T])>,
    collapsed: &HashSet<String>,
) -> Vec<Row<T>> {
    let mut rows = Vec::new();
    for (name, items) in groups {
        rows.push(Row::Group {
            name: name.to_string(),
        });
        if collapsed.contains(name) {
            continue;
        }
        rows.extend(items.iter().map(|&item| Row::Item {
            item,
            group: name.to_string(),
        }));
    }
    rows
}

/// The next selectable index in `dir`, skipping headers. Does not wrap: in a
/// long list, arrowing off the end and reappearing at the top loses the
/// reader's place more often than it helps.
pub fn next_row<T>(rows: &[Row<T>], from: isize, dir: isize) -> isize {
    let n = rows.len() as isize;
    if n == 0 {
        return -1;
    }
    let selectable = |i: isize| rows[i as usize].is_item();
    let mut i = from;
    for _ in 0..n {
        i += dir;
        if i < 0 || i >= n {
            return if from >= 0 && from < n && selectable(from) {
                from
            } else {
                -1
            };
        }
        if selectable(i) {
            return i;
        }
    }
    from
}

pub fn first_row<T>(rows: &[Row<T>]) -> isize {
    next_row(rows, -1, 1)
}

pub fn last_row<T>(rows: &[Row<T>]) -> isize {
    next_row(rows, rows.len() as isize, -1)
}

/// A page of rows, for PageUp/PageDown.
pub fn page_row<T>(rows: &[Row<T>], from: isize, dir: isize, page_size: usize) -> isize {
    let mut i = from;
    for _ in 0..page_size {
        let next = next_row(rows, i, dir);
        if next == i {
            break;
        }
        i = next;
    }
    i
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NavIntent {
    Move(isize),
    First,
    Last,
    Page(isize),
    Activate,
    Delete,
    Expand,
    Collapse,
    ToParent,
}

/// What a key should do, or `None` when it is not ours. An intent rather than
/// an index, so the caller performs the tree operations it alone knows.
/// `key` uses the DOM names the JS used; [`dom_key`] maps GPUI's.
pub fn nav_intent(key: &str, on_group: bool, collapsed: bool) -> Option<NavIntent> {
    Some(match key {
        "ArrowDown" => NavIntent::Move(1),
        "ArrowUp" => NavIntent::Move(-1),
        "Home" => NavIntent::First,
        "End" => NavIntent::Last,
        "PageDown" => NavIntent::Page(1),
        "PageUp" => NavIntent::Page(-1),
        "Enter" => NavIntent::Activate,
        "Delete" => NavIntent::Delete,
        // On a collapsed group, open it; anywhere else this is not ours, so a
        // text cursor in the search box still behaves normally.
        "ArrowRight" if on_group && collapsed => NavIntent::Expand,
        "ArrowRight" => return None,
        "ArrowLeft" if on_group && !collapsed => NavIntent::Collapse,
        "ArrowLeft" => NavIntent::ToParent,
        _ => return None,
    })
}

/// GPUI's keystroke key name as the DOM `key` the rules above are written in.
pub fn dom_key(gpui_key: &str) -> Option<&'static str> {
    Some(match gpui_key {
        "down" => "ArrowDown",
        "up" => "ArrowUp",
        "left" => "ArrowLeft",
        "right" => "ArrowRight",
        "home" => "Home",
        "end" => "End",
        "pagedown" => "PageDown",
        "pageup" => "PageUp",
        "enter" => "Enter",
        "delete" => "Delete",
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn groups() -> Vec<(&'static str, &'static [i64])> {
        vec![("web", &[1, 2]), ("db", &[3])]
    }
    fn kinds<T>(rows: &[Row<T>]) -> Vec<&'static str> {
        rows.iter()
            .map(|r| if r.is_item() { "host" } else { "group" })
            .collect()
    }
    fn none() -> HashSet<String> {
        HashSet::new()
    }

    #[test]
    fn interleaves_group_headers_with_their_hosts() {
        let rows = visible_rows(groups(), &none());
        assert_eq!(kinds(&rows), ["group", "host", "host", "group", "host"]);
    }

    #[test]
    fn omits_the_rows_of_a_collapsed_group_entirely() {
        let collapsed = HashSet::from(["web".to_string()]);
        let rows = visible_rows(groups(), &collapsed);
        assert_eq!(kinds(&rows), ["group", "group", "host"]);
        assert_eq!(rows.iter().filter(|r| r.is_item()).count(), 1);
    }

    #[test]
    fn survives_empty_input() {
        let empty: Vec<(&str, &[i64])> = vec![];
        assert!(visible_rows(empty, &none()).is_empty());
        let header_only: Vec<(&str, &[i64])> = vec![("x", &[])];
        assert_eq!(visible_rows(header_only, &none()).len(), 1);
    }

    #[test]
    fn skips_group_headers() {
        let rows = visible_rows(groups(), &none());
        assert_eq!(next_row(&rows, 1, 1), 2);
        assert_eq!(next_row(&rows, 2, 1), 4);
        assert_eq!(next_row(&rows, 4, -1), 2);
    }

    #[test]
    fn stops_at_the_ends_rather_than_wrapping() {
        let rows = visible_rows(groups(), &none());
        assert_eq!(next_row(&rows, 4, 1), 4);
        assert_eq!(next_row(&rows, 1, -1), 1);
    }

    #[test]
    fn finds_the_first_and_last_selectable_rows() {
        let rows = visible_rows(groups(), &none());
        assert_eq!(first_row(&rows), 1);
        assert_eq!(last_row(&rows), 4);
    }

    #[test]
    fn returns_minus_one_when_nothing_is_selectable() {
        let only_headers: Vec<(&str, &[i64])> = vec![("a", &[]), ("b", &[])];
        let rows = visible_rows(only_headers, &none());
        assert_eq!(first_row(&rows), -1);
        let empty: Vec<Row<i64>> = vec![];
        assert_eq!(next_row(&empty, 0, 1), -1);
    }

    #[test]
    fn moves_a_page_and_stops_at_the_end() {
        let ids: Vec<i64> = (0..30).collect();
        let many = visible_rows([("g", ids.as_slice())], &none());
        assert_eq!(page_row(&many, 1, 1, 10), 11);
        assert_eq!(page_row(&many, 25, 1, 10), 30);
        assert_eq!(page_row(&many, 5, -1, 10), 1);
    }

    #[test]
    fn maps_the_movement_keys() {
        assert_eq!(
            nav_intent("ArrowDown", false, false),
            Some(NavIntent::Move(1))
        );
        assert_eq!(
            nav_intent("ArrowUp", false, false),
            Some(NavIntent::Move(-1))
        );
        assert_eq!(nav_intent("Home", false, false), Some(NavIntent::First));
        assert_eq!(nav_intent("End", false, false), Some(NavIntent::Last));
        assert_eq!(nav_intent("Enter", false, false), Some(NavIntent::Activate));
    }

    #[test]
    fn treats_the_arrows_as_tree_controls_only_on_a_group() {
        assert_eq!(
            nav_intent("ArrowRight", true, true),
            Some(NavIntent::Expand)
        );
        assert_eq!(
            nav_intent("ArrowLeft", true, false),
            Some(NavIntent::Collapse)
        );
        // Already expanded: nothing to open.
        assert_eq!(nav_intent("ArrowRight", true, false), None);
    }

    #[test]
    fn leaves_right_arrow_alone_on_a_host_so_a_text_cursor_still_works() {
        assert_eq!(nav_intent("ArrowRight", false, false), None);
    }

    #[test]
    fn claims_nothing_it_does_not_own() {
        for k in ["a", "Tab", "Shift", "F5", " "] {
            assert_eq!(nav_intent(k, false, false), None, "{k:?}");
        }
    }

    #[test]
    fn host_row_height_matches_the_row_style() {
        // One constant, so a virtual list and the style cannot disagree.
        assert_eq!(HOST_ROW_H, 32.0);
    }

    #[test]
    fn maps_gpui_key_names() {
        assert_eq!(dom_key("down"), Some("ArrowDown"));
        assert_eq!(dom_key("pageup"), Some("PageUp"));
        assert_eq!(dom_key("a"), None);
    }
}
