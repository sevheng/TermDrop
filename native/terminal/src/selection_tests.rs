//! Word and line selection, checked against a real `Term`: double-click is a
//! semantic selection and triple-click a line selection, exactly as the view
//! creates them.

use alacritty_terminal::event::VoidListener;
use alacritty_terminal::index::{Column, Line, Point, Side};
use alacritty_terminal::selection::{Selection, SelectionType};
use alacritty_terminal::term::{Config, Term};
use alacritty_terminal::vte::ansi::Processor;

use crate::feed::GridSize;

fn term_with(text: &str, cols: usize) -> Term<VoidListener> {
    let mut term = Term::new(
        Config::default(),
        &GridSize { cols, lines: 4 },
        VoidListener,
    );
    let mut parser: Processor = Processor::new();
    parser.advance(&mut term, text.as_bytes());
    term
}

fn select(term: &mut Term<VoidListener>, ty: SelectionType, line: i32, col: usize) -> String {
    let point = Point::new(Line(line), Column(col));
    term.selection = Some(Selection::new(ty, point, Side::Left));
    term.selection_to_string().unwrap_or_default()
}

#[test]
fn double_click_selects_a_path_as_one_word() {
    let mut term = term_with("cat /var/log/syslog.1 now", 40);
    assert_eq!(
        select(&mut term, SelectionType::Semantic, 0, 8),
        "/var/log/syslog.1"
    );
    assert_eq!(select(&mut term, SelectionType::Semantic, 0, 0), "cat");
}

#[test]
fn quotes_brackets_and_commas_end_a_word() {
    let mut term = term_with("a='b' (c) x,y [z]", 40);
    assert_eq!(select(&mut term, SelectionType::Semantic, 0, 3), "b");
    assert_eq!(select(&mut term, SelectionType::Semantic, 0, 7), "c");
    assert_eq!(select(&mut term, SelectionType::Semantic, 0, 10), "x");
    assert_eq!(select(&mut term, SelectionType::Semantic, 0, 15), "z");
}

#[test]
fn double_click_follows_a_word_across_a_soft_wrap() {
    // 10 columns: "echo abcdefghij" wraps inside the second word.
    let mut term = term_with("echo abcdefghij", 10);
    assert_eq!(
        select(&mut term, SelectionType::Semantic, 0, 6),
        "abcdefghij"
    );
}

#[test]
fn triple_click_selects_the_whole_line() {
    let mut term = term_with("first line\r\nsecond line", 40);
    // A line selection carries its line break, so pasting it runs it.
    assert_eq!(
        select(&mut term, SelectionType::Lines, 1, 3),
        "second line\n"
    );
}
