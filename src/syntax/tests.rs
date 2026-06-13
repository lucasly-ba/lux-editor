//! Tests for syntax highlighting and the incremental-edit diff.

use super::*;

#[test]
fn diff_of_identical_text_is_none() {
    assert!(diff_input_edit("hello", "hello").is_none());
}

#[test]
fn diff_finds_a_middle_insertion() {
    // "abXYc" inserts "XY" after "ab".
    let edit = diff_input_edit("abc", "abXYc").unwrap();
    assert_eq!(edit.start_byte, 2);
    assert_eq!(edit.old_end_byte, 2); // nothing removed
    assert_eq!(edit.new_end_byte, 4); // "XY" added
}

#[test]
fn diff_finds_a_deletion() {
    let edit = diff_input_edit("hello world", "hello").unwrap();
    assert_eq!(edit.start_byte, 5);
    assert_eq!(edit.old_end_byte, 11);
    assert_eq!(edit.new_end_byte, 5);
}

#[test]
fn point_tracks_rows_and_columns() {
    let s = "ab\ncde\nf";
    assert_eq!(point_at(s, 0), Point { row: 0, column: 0 });
    assert_eq!(point_at(s, 4), Point { row: 1, column: 1 }); // 'd'
    assert_eq!(point_at(s, 7), Point { row: 2, column: 0 }); // 'f'
}

#[test]
fn highlights_rust_keywords() {
    let rope = Rope::from_str("fn main() {\n    let x = 42;\n}\n");
    let mut hl = Highlighter::rust();
    hl.reparse(&rope);
    let spans = hl.spans(&rope, 0, 10);
    assert!(!spans.is_empty(), "expected some highlight spans for Rust code");

    // The `fn` keyword at chars 0..2 should be coloured.
    assert!(
        spans.iter().any(|s| s.start == 0 && s.end == 2),
        "expected a span over the `fn` keyword, got {:?}",
        spans.iter().map(|s| (s.start, s.end)).collect::<Vec<_>>()
    );
}

#[test]
fn incremental_reparse_stays_consistent() {
    // Parse, then edit, then re-parse; the highlighter should track the change
    // without panicking and still produce spans for the new text.
    let mut rope = Rope::from_str("fn main() {}\n");
    let mut hl = Highlighter::rust();
    hl.reparse(&rope);

    // Insert a `let` binding inside the body.
    rope.insert(11, "\n    let y = 1;\n");
    hl.reparse(&rope);

    assert_eq!(hl.text, rope.to_string());
    let spans = hl.spans(&rope, 0, 10);
    assert!(!spans.is_empty());
}
