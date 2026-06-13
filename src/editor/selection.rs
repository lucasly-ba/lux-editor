//! Helpers for the visual-mode selection.
//!
//! A selection is just two points: the **anchor** (fixed when the selection
//! started) and the **head** (the moving cursor). The selected text is whatever
//! lies between them, regardless of which one comes first.

use std::ops::Range;

use crate::text::{Buffer, Position};

/// The half-open character range covered by a selection from `anchor` to
/// `head`, inclusive of the character under whichever end is last.
///
/// Visual mode in Vim/Helix selects the character the cursor is *on*, so a
/// selection where anchor == head still covers one character. The returned
/// range is clamped to the buffer length.
pub fn selection_range(buffer: &Buffer, anchor: Position, head: Position) -> Range<usize> {
    let a = buffer.position_to_char(anchor);
    let h = buffer.position_to_char(head);
    let start = a.min(h);
    // Include the character under the later endpoint.
    let end = (a.max(h) + 1).min(buffer.len_chars());
    start..end.max(start)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn covers_inclusive_range() {
        let buf = Buffer::from_string("hello world");
        // anchor on 'h' (0,0), head on 'o' (0,4) -> "hello"
        let r = selection_range(&buf, Position::new(0, 0), Position::new(0, 4));
        assert_eq!(&buf.rope().to_string()[r], "hello");
    }

    #[test]
    fn order_independent() {
        let buf = Buffer::from_string("hello world");
        let forward = selection_range(&buf, Position::new(0, 0), Position::new(0, 4));
        let backward = selection_range(&buf, Position::new(0, 4), Position::new(0, 0));
        assert_eq!(forward, backward);
    }

    #[test]
    fn single_char_when_collapsed() {
        let buf = Buffer::from_string("abc");
        let r = selection_range(&buf, Position::new(0, 1), Position::new(0, 1));
        assert_eq!(&buf.rope().to_string()[r], "b");
    }
}
