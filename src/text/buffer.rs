//! A `Buffer`: a rope plus the metadata an editor needs around it.

use std::io;
use std::path::{Path, PathBuf};

use super::edit::Edit;
use super::position::Position;
use crate::rope::Rope;

/// An open document: the text (as a [`Rope`]), where it came from, and whether
/// it has unsaved changes.
///
/// The buffer is the single place that mutates the text. Every change goes
/// through [`Buffer::apply`] and produces an [`Edit`], which is what makes
/// undo, incremental re-parsing and LSP synchronisation possible: they all just
/// watch the stream of edits.
pub struct Buffer {
    rope: Rope,
    path: Option<PathBuf>,
    modified: bool,
    /// Monotonic document version, bumped on every edit. The LSP client reports
    /// this to the language server so it knows which change it is looking at.
    version: i32,
}

impl Buffer {
    /// An empty, untitled buffer.
    pub fn new() -> Buffer {
        Buffer {
            rope: Rope::new(),
            path: None,
            modified: false,
            version: 0,
        }
    }

    /// A buffer holding `text`, untitled.
    pub fn from_string(text: &str) -> Buffer {
        Buffer {
            rope: Rope::from_str(text),
            path: None,
            modified: false,
            version: 0,
        }
    }

    /// Load a buffer from a file. A missing file yields an empty buffer that
    /// remembers the path, so saving will create it (like `vim file-that-does-not-exist`).
    pub fn from_file(path: impl AsRef<Path>) -> io::Result<Buffer> {
        let path = path.as_ref().to_path_buf();
        let text = match std::fs::read_to_string(&path) {
            Ok(text) => text,
            Err(e) if e.kind() == io::ErrorKind::NotFound => String::new(),
            Err(e) => return Err(e),
        };
        Ok(Buffer {
            rope: Rope::from_str(&text),
            path: Some(path),
            modified: false,
            version: 0,
        })
    }

    /// Write the buffer back to its path. Errors if the buffer is untitled.
    pub fn save(&mut self) -> io::Result<()> {
        let path = self
            .path
            .clone()
            .ok_or_else(|| io::Error::new(io::ErrorKind::Other, "buffer has no path"))?;
        self.save_as(path)
    }

    /// Write the buffer to `path` and adopt it as the buffer's path.
    pub fn save_as(&mut self, path: impl AsRef<Path>) -> io::Result<()> {
        let path = path.as_ref().to_path_buf();
        std::fs::write(&path, self.rope.to_string())?;
        self.path = Some(path);
        self.modified = false;
        Ok(())
    }

    // --- read-only accessors ------------------------------------------------

    pub fn rope(&self) -> &Rope {
        &self.rope
    }

    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    pub fn is_modified(&self) -> bool {
        self.modified
    }

    pub fn version(&self) -> i32 {
        self.version
    }

    /// A short display name for the status line.
    pub fn display_name(&self) -> String {
        self.path
            .as_ref()
            .and_then(|p| p.file_name())
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "[scratch]".to_string())
    }

    pub fn len_chars(&self) -> usize {
        self.rope.len_chars()
    }

    pub fn len_lines(&self) -> usize {
        self.rope.len_lines()
    }

    /// Text of `line` (0-based), including its trailing newline if present.
    pub fn line(&self, line: usize) -> String {
        self.rope.line(line)
    }

    /// Number of characters on `line`, excluding the trailing newline. This is
    /// the count a cursor cares about: the rightmost legal column.
    pub fn line_width(&self, line: usize) -> usize {
        let text = self.rope.line(line);
        let trimmed = text.strip_suffix('\n').unwrap_or(&text);
        trimmed.chars().count()
    }

    // --- coordinate conversions ---------------------------------------------

    /// Convert a `(line, column)` position into a flat character index,
    /// clamping the position into range first.
    pub fn position_to_char(&self, pos: Position) -> usize {
        let last_line = self.len_lines().saturating_sub(1);
        let line = pos.line.min(last_line);
        let line_start = self.rope.line_to_char(line);
        let column = pos.column.min(self.line_width(line));
        line_start + column
    }

    /// Convert a flat character index into a `(line, column)` position.
    pub fn char_to_position(&self, idx: usize) -> Position {
        let idx = idx.min(self.len_chars());
        let line = self.rope.char_to_line(idx);
        let column = idx - self.rope.line_to_char(line);
        Position { line, column }
    }

    // --- mutation -----------------------------------------------------------

    /// Apply an edit to the text and return it (so callers can record it).
    ///
    /// `edit.removed` is assumed to match what is currently at `edit.at`; this
    /// holds for edits produced by the buffer itself and for the inverses used
    /// by undo/redo.
    pub fn apply(&mut self, edit: Edit) -> Edit {
        if !edit.removed.is_empty() {
            self.rope.remove(edit.at..edit.at + edit.removed_chars());
        }
        if !edit.inserted.is_empty() {
            self.rope.insert(edit.at, &edit.inserted);
        }
        self.modified = true;
        self.version += 1;
        edit
    }

    /// Insert `text` at character index `at`, returning the resulting edit.
    pub fn insert(&mut self, at: usize, text: &str) -> Edit {
        self.apply(Edit::insertion(at, text))
    }

    /// Remove the characters in `range`, returning the resulting edit (which
    /// captures the removed text so it can be undone).
    pub fn remove(&mut self, range: std::ops::Range<usize>) -> Edit {
        let removed = self.rope.slice(range.clone());
        self.apply(Edit::deletion(range.start, removed))
    }

    /// Replace the characters in `range` with `text` in one edit.
    pub fn replace(&mut self, range: std::ops::Range<usize>, text: &str) -> Edit {
        let removed = self.rope.slice(range.clone());
        self.apply(Edit {
            at: range.start,
            removed,
            inserted: text.to_string(),
        })
    }
}

impl Default for Buffer {
    fn default() -> Buffer {
        Buffer::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn insert_and_remove_track_modified() {
        let mut b = Buffer::from_string("hello");
        assert!(!b.is_modified());
        b.insert(5, " world");
        assert_eq!(b.rope().to_string(), "hello world");
        assert!(b.is_modified());
        assert_eq!(b.version(), 1);
        b.remove(0..6);
        assert_eq!(b.rope().to_string(), "world");
        assert_eq!(b.version(), 2);
    }

    #[test]
    fn edit_is_reversible() {
        let mut b = Buffer::from_string("abcdef");
        let edit = b.remove(2..4); // remove "cd"
        assert_eq!(b.rope().to_string(), "abef");
        // Applying the inverse restores the original text.
        b.apply(edit.inverse());
        assert_eq!(b.rope().to_string(), "abcdef");
    }

    #[test]
    fn replace_in_one_edit() {
        let mut b = Buffer::from_string("the quick fox");
        let edit = b.replace(4..9, "slow");
        assert_eq!(b.rope().to_string(), "the slow fox");
        assert_eq!(edit.removed, "quick");
        assert_eq!(edit.inserted, "slow");
    }

    #[test]
    fn position_round_trips() {
        let b = Buffer::from_string("first\nsecond\nthird");
        for (idx, line, col) in [(0, 0, 0), (3, 0, 3), (6, 1, 0), (13, 2, 0), (18, 2, 5)] {
            assert_eq!(b.char_to_position(idx), Position::new(line, col));
            assert_eq!(b.position_to_char(Position::new(line, col)), idx);
        }
    }

    #[test]
    fn position_clamps_out_of_range() {
        let b = Buffer::from_string("ab\ncd");
        // Column past end of line clamps to the line width.
        assert_eq!(b.position_to_char(Position::new(0, 99)), 2);
        // Line past end clamps to the last line.
        assert_eq!(b.position_to_char(Position::new(99, 0)), 3);
    }

    #[test]
    fn line_width_excludes_newline() {
        let b = Buffer::from_string("abc\nde\n");
        assert_eq!(b.line_width(0), 3);
        assert_eq!(b.line_width(1), 2);
        assert_eq!(b.line_width(2), 0); // empty last line
    }
}
