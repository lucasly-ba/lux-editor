//! Syntax highlighting with **tree-sitter**, parsed **incrementally**.
//!
//! Two Tier-1 ideas live here:
//!
//! 1. **A real parser, not regex.** lux uses [tree-sitter] (the same parser
//!    generator Helix and Neovim use) to build a concrete syntax tree of the
//!    code, then runs the grammar's highlight query over it to colour tokens.
//!    Because it understands the grammar, it gets things regex can't: nested
//!    strings, raw identifiers, generics, and so on.
//!
//! 2. **Incremental re-parsing.** Re-parsing the whole file on every keystroke
//!    would be wasteful. After an edit, lux computes the *minimal* changed byte
//!    range (the text between the common prefix and common suffix of the old and
//!    new contents), tells tree-sitter about it via `Tree::edit`, and re-parses
//!    feeding the *old* tree back in. tree-sitter then reuses every unchanged
//!    subtree and only does work proportional to the edit, not the file.
//!
//! [tree-sitter]: https://tree-sitter.github.io/

use std::path::Path;

use tree_sitter::{InputEdit, Parser, Point, Query, QueryCursor, Tree};

use crate::rope::Rope;
use crate::ui::HighlightSpan;
use crate::ui::theme::highlight_color;

/// Owns a tree-sitter parser, the current syntax tree, and a snapshot of the
/// text the tree corresponds to. One per highlighted buffer.
pub struct Highlighter {
    parser: Parser,
    query: Query,
    tree: Option<Tree>,
    /// The text the current `tree` was parsed from. Kept so the next edit can be
    /// diffed against it to find the minimal changed range.
    text: String,
}

impl Highlighter {
    /// A highlighter for `path`, if lux knows the language for its extension.
    pub fn for_path(path: &Path) -> Option<Highlighter> {
        match path.extension().and_then(|e| e.to_str()) {
            Some("rs") => Some(Highlighter::rust()),
            _ => None,
        }
    }

    /// A highlighter for Rust.
    pub fn rust() -> Highlighter {
        let language = tree_sitter_rust::language();
        let mut parser = Parser::new();
        parser
            .set_language(&language)
            .expect("the bundled Rust grammar is compatible");
        let query = Query::new(&language, tree_sitter_rust::HIGHLIGHTS_QUERY)
            .expect("the bundled Rust highlight query parses");
        Highlighter {
            parser,
            query,
            tree: None,
            text: String::new(),
        }
    }

    /// Re-parse after the buffer changed.
    ///
    /// Diffs the new text against the previous snapshot to find the changed
    /// range, applies it to the old tree as an [`InputEdit`], and re-parses
    /// incrementally. The first call (no previous tree) is a full parse.
    pub fn reparse(&mut self, rope: &Rope) {
        let new_text = rope.to_string();

        if let Some(tree) = &mut self.tree
            && let Some(edit) = diff_input_edit(&self.text, &new_text)
        {
            tree.edit(&edit);
        }
        self.text = new_text;

        let old_tree = self.tree.take();
        // Split borrows explicitly so the read callback can reference `text`
        // while the parser is borrowed mutably.
        let parser = &mut self.parser;
        let text = &self.text;
        self.tree = parser.parse_with(
            &mut |byte, _point| {
                let bytes = text.as_bytes();
                &bytes[byte.min(bytes.len())..]
            },
            old_tree.as_ref(),
        );
    }

    /// Highlight spans for the visible lines `[scroll, scroll + height)`.
    ///
    /// Returned spans use **character** indices (what the renderer works in),
    /// converted from tree-sitter's byte offsets. Only the visible region is
    /// queried, so this stays cheap regardless of file size.
    pub fn spans(&self, rope: &Rope, scroll: usize, height: usize) -> Vec<HighlightSpan> {
        let Some(tree) = &self.tree else {
            return Vec::new();
        };
        let total_lines = rope.len_lines();
        let end_line = (scroll + height).min(total_lines);
        if scroll >= end_line {
            return Vec::new();
        }

        let start_char = rope.line_to_char(scroll);
        let end_char = rope.line_to_char(end_line);
        let start_byte = char_to_byte(&self.text, start_char);
        let end_byte = char_to_byte(&self.text, end_char);

        let mut cursor = QueryCursor::new();
        cursor.set_byte_range(start_byte..end_byte);

        // Collect every capture overlapping the visible window.
        let names = self.query.capture_names();
        let mut captures: Vec<(usize, usize, &str)> = Vec::new();
        for m in cursor.matches(&self.query, tree.root_node(), self.text.as_bytes()) {
            for cap in m.captures {
                let node = cap.node;
                captures.push((
                    node.start_byte(),
                    node.end_byte(),
                    names[cap.index as usize],
                ));
            }
        }

        // Paint a per-character colour buffer for the window. Processing the
        // largest nodes first means the smallest (most specific) capture wins
        // wherever they overlap, e.g. a function name inside an expression.
        captures.sort_by_key(|(s, e, _)| std::cmp::Reverse(e - s));
        let span_chars = end_char - start_char;
        let mut colors = vec![None; span_chars];
        for (sb, eb, name) in captures {
            let Some(color) = highlight_color(name) else {
                continue;
            };
            let sb = sb.max(start_byte);
            let eb = eb.min(end_byte);
            if sb >= eb {
                continue;
            }
            let cs = self.text[start_byte..sb].chars().count();
            let ce = cs + self.text[sb..eb].chars().count();
            for slot in &mut colors[cs..ce] {
                *slot = Some(color);
            }
        }

        // Coalesce equal-coloured runs into spans (absolute character indices).
        let mut spans = Vec::new();
        let mut i = 0;
        while i < span_chars {
            let Some(color) = colors[i] else {
                i += 1;
                continue;
            };
            let mut j = i + 1;
            while j < span_chars && colors[j] == Some(color) {
                j += 1;
            }
            spans.push(HighlightSpan {
                start: start_char + i,
                end: start_char + j,
                color,
            });
            i = j;
        }
        spans
    }
}

/// Byte offset of character index `char_idx` within `s`, clamped to the end.
fn char_to_byte(s: &str, char_idx: usize) -> usize {
    s.char_indices()
        .nth(char_idx)
        .map(|(b, _)| b)
        .unwrap_or(s.len())
}

/// Compute the minimal [`InputEdit`] describing the change from `old` to `new`,
/// or `None` if they are identical.
///
/// The changed region is everything between the longest common prefix and the
/// longest common suffix of the two strings, snapped to character boundaries.
fn diff_input_edit(old: &str, new: &str) -> Option<InputEdit> {
    if old == new {
        return None;
    }
    let (ob, nb) = (old.as_bytes(), new.as_bytes());

    // Longest common prefix, backed off to a char boundary.
    let mut start = 0;
    let max_prefix = ob.len().min(nb.len());
    while start < max_prefix && ob[start] == nb[start] {
        start += 1;
    }
    while start > 0 && !old.is_char_boundary(start) {
        start -= 1;
    }

    // Longest common suffix that doesn't overlap the prefix, on a char boundary.
    // (The two sides index `ob`/`nb` from their own ends on purpose: the
    // strings differ in length, so clippy's "use the same len" hint is wrong.)
    let mut suffix = 0;
    let max_suffix = (ob.len() - start).min(nb.len() - start);
    #[allow(clippy::suspicious_operation_groupings)]
    while suffix < max_suffix && ob[ob.len() - 1 - suffix] == nb[nb.len() - 1 - suffix] {
        suffix += 1;
    }
    while suffix > 0 && !old.is_char_boundary(old.len() - suffix) {
        suffix -= 1;
    }

    let old_end = old.len() - suffix;
    let new_end = new.len() - suffix;

    Some(InputEdit {
        start_byte: start,
        old_end_byte: old_end,
        new_end_byte: new_end,
        start_position: point_at(old, start),
        old_end_position: point_at(old, old_end),
        new_end_position: point_at(new, new_end),
    })
}

/// The tree-sitter [`Point`] (row, byte-column) of byte offset `byte` in `s`.
fn point_at(s: &str, byte: usize) -> Point {
    let prefix = &s[..byte];
    let row = prefix.bytes().filter(|&b| b == b'\n').count();
    let column = prefix.len() - prefix.rfind('\n').map(|i| i + 1).unwrap_or(0);
    Point { row, column }
}

#[cfg(test)]
mod tests;
