//! Find in the terminal: the port of xterm.js's search addon as the Tauri
//! app used it (`findNext` / `findPrevious` with a plain-text query and a
//! case-sensitivity flag, no regex, no whole-word).
//!
//! The query is a literal, escaped into a pattern for `alacritty_terminal`'s
//! regex search, which runs over the whole grid: scrollback and screen. As in
//! xterm.js, the match found becomes the selection (so Copy takes it), the
//! next search starts from the selection, the search wraps at either end, and
//! the view scrolls to show the match.

use alacritty_terminal::event::EventListener;
use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::index::{Boundary, Column, Direction, Line, Point, Side};
use alacritty_terminal::selection::{Selection, SelectionType};
use alacritty_terminal::term::search::{Match, RegexIter, RegexSearch};
use alacritty_terminal::term::Term;

/// Matches highlighted on screen at most. A one-letter query on a full
/// screen of text is a few thousand; past this the highlights stop rather
/// than the frame.
pub(crate) const MAX_VISIBLE_MATCHES: usize = 2000;

/// Which way `find` goes: `findNext` or `findPrevious`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SearchDirection {
    Next,
    Previous,
}

/// The regex for a literal `query`. `alacritty_terminal` is smart-case on
/// its own (case-insensitive unless the pattern has a capital); the inline
/// flag overrides that, so the "Aa" box alone decides, as in xterm.js.
pub(crate) fn pattern(query: &str, case_sensitive: bool) -> Option<String> {
    if query.is_empty() {
        return None;
    }
    let flag = if case_sensitive { "(?-i)" } else { "(?i)" };
    Some(format!("{flag}{}", regex::escape(query)))
}

/// A compiled query, kept so that stepping through matches does not rebuild
/// the automata, and so the painter can highlight the visible matches.
pub(crate) struct ActiveSearch {
    pub(crate) pattern: String,
    pub(crate) regex: RegexSearch,
}

impl ActiveSearch {
    pub(crate) fn new(query: &str, case_sensitive: bool) -> Option<Self> {
        let pattern = pattern(query, case_sensitive)?;
        let regex = RegexSearch::new(&pattern).ok()?;
        Some(Self { pattern, regex })
    }
}

/// Finds the next match from the selection (or from the top of the
/// scrollback for Next, the bottom of the screen for Previous, when nothing
/// is selected), wrapping around, selects it and scrolls it into view.
/// Returns whether anything matched; on no match the selection is left as
/// it was, as xterm.js leaves it.
pub(crate) fn find<T: EventListener>(
    term: &mut Term<T>,
    regex: &mut RegexSearch,
    direction: SearchDirection,
) -> Option<Match> {
    let selection = term.selection.as_ref().and_then(|s| s.to_range(term));
    let found = match direction {
        SearchDirection::Next => {
            let origin = match selection {
                Some(range) => range.end.add(term, Boundary::None, 1),
                None => Point::new(term.topmost_line(), Column(0)),
            };
            term.search_next(regex, origin, Direction::Right, Side::Left, None)
        }
        SearchDirection::Previous => {
            let origin = match selection {
                Some(range) => range.start.sub(term, Boundary::None, 1),
                None => Point::new(term.bottommost_line(), term.last_column()),
            };
            term.search_next(regex, origin, Direction::Left, Side::Right, None)
        }
    }?;
    select_range(term, &found);
    term.scroll_to_point(*found.start());
    Some(found)
}

/// Selects `range`, the way a drag from its first to its last cell would.
pub(crate) fn select_range<T>(term: &mut Term<T>, range: &Match) {
    let mut selection = Selection::new(SelectionType::Simple, *range.start(), Side::Left);
    selection.update(*range.end(), Side::Right);
    term.selection = Some(selection);
}

/// Selects everything: scrollback and screen (`term.selectAll()`).
pub(crate) fn select_all<T>(term: &mut Term<T>) {
    let start = Point::new(term.topmost_line(), Column(0));
    let end = Point::new(term.bottommost_line(), term.last_column());
    select_range(term, &(start..=end));
}

/// The matches on screen, as (viewport row, first column, last column)
/// runs, one per row a match covers.
pub(crate) fn visible_matches<T>(
    term: &Term<T>,
    regex: &mut RegexSearch,
) -> Vec<(usize, usize, usize)> {
    let offset = term.grid().display_offset() as i32;
    let rows = term.screen_lines() as i32;
    let last = term.last_column();
    let start = Point::new(Line(-offset), Column(0));
    let end = Point::new(Line(rows - 1 - offset), last);
    let mut runs = Vec::new();
    for m in RegexIter::new(start, end, Direction::Right, term, regex).take(MAX_VISIBLE_MATCHES) {
        let (s, e) = (*m.start(), *m.end());
        for line in s.line.0..=e.line.0 {
            let row = line + offset;
            if !(0..rows).contains(&row) {
                continue;
            }
            let first = if line == s.line.0 { s.column.0 } else { 0 };
            let last_col = if line == e.line.0 { e.column.0 } else { last.0 };
            runs.push((row as usize, first, last_col));
        }
    }
    runs
}

#[cfg(test)]
mod tests {
    use alacritty_terminal::event::VoidListener;
    use alacritty_terminal::term::Config;
    use alacritty_terminal::vte::ansi::Processor;

    use super::*;
    use crate::feed::GridSize;

    fn term_with(text: &str, cols: usize, lines: usize) -> Term<VoidListener> {
        let config = Config {
            scrolling_history: 100,
            ..Config::default()
        };
        let mut term = Term::new(config, &GridSize { cols, lines }, VoidListener);
        let mut parser: Processor = Processor::new();
        parser.advance(&mut term, text.as_bytes());
        term
    }

    fn search(query: &str, case_sensitive: bool) -> RegexSearch {
        ActiveSearch::new(query, case_sensitive).unwrap().regex
    }

    fn selected(term: &Term<VoidListener>) -> String {
        term.selection_to_string().unwrap_or_default()
    }

    #[test]
    fn the_query_is_a_literal() {
        assert_eq!(pattern("", false), None);
        assert_eq!(pattern("a.b", false).unwrap(), "(?i)a\\.b");
        assert_eq!(pattern("[x]*", true).unwrap(), "(?-i)\\[x\\]\\*");
        // A dot does not match any character.
        let mut term = term_with("axb a.b", 20, 2);
        let mut re = search("a.b", false);
        let m = find(&mut term, &mut re, SearchDirection::Next).unwrap();
        assert_eq!(m.start().column, Column(4));
    }

    #[test]
    fn the_case_box_alone_decides() {
        let mut term = term_with("Error error ERROR", 20, 2);
        // Lowercase query, case-sensitive: only the exact one.
        let mut re = search("error", true);
        let m = find(&mut term, &mut re, SearchDirection::Next).unwrap();
        assert_eq!(m.start().column, Column(6));
        let again = find(&mut term, &mut re, SearchDirection::Next).unwrap();
        assert_eq!(
            again.start().column,
            Column(6),
            "the only match wraps to itself"
        );
        // A capital in the query, case-insensitive: all three (alacritty's
        // smart case would have found one).
        term.selection = None;
        let mut re = search("Error", false);
        let cols: Vec<usize> = (0..3)
            .map(|_| {
                find(&mut term, &mut re, SearchDirection::Next)
                    .unwrap()
                    .start()
                    .column
                    .0
            })
            .collect();
        assert_eq!(cols, vec![0, 6, 12]);
    }

    #[test]
    fn next_and_previous_step_from_the_selection_and_wrap() {
        let mut term = term_with("one foo\r\ntwo foo\r\nthree foo", 20, 4);
        let mut re = search("foo", false);
        let lines: Vec<i32> = (0..4)
            .map(|_| {
                find(&mut term, &mut re, SearchDirection::Next)
                    .unwrap()
                    .start()
                    .line
                    .0
            })
            .collect();
        assert_eq!(lines, vec![0, 1, 2, 0]);
        assert_eq!(selected(&term), "foo");
        let back = find(&mut term, &mut re, SearchDirection::Previous).unwrap();
        assert_eq!(back.start().line, Line(2), "previous wraps to the end");
        let back = find(&mut term, &mut re, SearchDirection::Previous).unwrap();
        assert_eq!(back.start().line, Line(1));
    }

    #[test]
    fn previous_with_no_selection_starts_at_the_bottom() {
        let mut term = term_with("foo 1\r\nfoo 2\r\nfoo 3", 20, 4);
        let mut re = search("foo", false);
        let m = find(&mut term, &mut re, SearchDirection::Previous).unwrap();
        assert_eq!(m.start().line, Line(2));
    }

    #[test]
    fn no_match_leaves_the_selection_alone() {
        let mut term = term_with("hello world", 20, 2);
        let mut re = search("world", false);
        find(&mut term, &mut re, SearchDirection::Next).unwrap();
        let mut missing = search("nothing", false);
        assert!(find(&mut term, &mut missing, SearchDirection::Next).is_none());
        assert_eq!(selected(&term), "world");
    }

    #[test]
    fn a_match_in_the_scrollback_is_found_and_scrolled_to() {
        let mut text = String::from("needle\r\n");
        for i in 0..30 {
            text.push_str(&format!("line {i}\r\n"));
        }
        let mut term = term_with(&text, 20, 5);
        assert_eq!(term.grid().display_offset(), 0);
        let mut re = search("needle", false);
        let m = find(&mut term, &mut re, SearchDirection::Next).unwrap();
        assert!(m.start().line.0 < 0, "the match is in the history");
        assert!(
            term.grid().display_offset() > 0,
            "the view scrolled up to it"
        );
        // Now on screen, so it is among the visible matches.
        let runs = visible_matches(&term, &mut re);
        assert_eq!(runs.len(), 1);
        assert_eq!((runs[0].1, runs[0].2), (0, 5));
    }

    #[test]
    fn visible_matches_split_a_wrapped_match_by_row() {
        // 10 columns: "xxxxxxxabcdef" wraps after "abc".
        let term = term_with("xxxxxxxabcdef", 10, 3);
        let mut re = search("abcdef", false);
        assert_eq!(visible_matches(&term, &mut re), vec![(0, 7, 9), (1, 0, 2)]);
    }

    #[test]
    fn select_all_takes_the_history_too() {
        let mut text = String::new();
        for i in 0..10 {
            text.push_str(&format!("row{i}\r\n"));
        }
        let mut term = term_with(&text, 10, 3);
        select_all(&mut term);
        let all = selected(&term);
        assert!(all.starts_with("row0\n"), "{all:?}");
        assert!(all.contains("row9"));
    }
}
