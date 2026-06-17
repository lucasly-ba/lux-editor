//! The `Editor`: the modal state machine that ties the buffer, history and
//! cursor together.
//!
//! Everything the user can do is expressed as an [`Action`]. The input layer
//! turns key presses into actions (depending on the current [`Mode`]), and
//! [`Editor::apply_action`] is the single place those actions are interpreted.
//! Keeping all editing logic behind `apply_action`, with no terminal or
//! rendering in sight, is what makes the editor unit-testable.

mod mode;
mod selection;

pub use mode::Mode;

use crate::history::History;
use crate::text::{Buffer, Edit, Position};

/// Character class, used by word-wise motions to decide where a "word" ends.
#[derive(PartialEq, Eq, Clone, Copy)]
enum Class {
    Whitespace,
    Word,
    Punctuation,
}

fn class_of(c: char) -> Class {
    if c.is_whitespace() {
        Class::Whitespace
    } else if c.is_alphanumeric() || c == '_' {
        Class::Word
    } else {
        Class::Punctuation
    }
}

/// A single thing the editor can be asked to do. Produced by the input layer,
/// consumed by [`Editor::apply_action`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    // Motions
    MoveLeft,
    MoveRight,
    MoveUp,
    MoveDown,
    MoveLineStart,
    MoveLineEnd,
    MoveWordForward,
    MoveWordBackward,
    MoveBufferStart,
    MoveBufferEnd,
    // Mode changes / insertion entry points
    EnterInsert,
    InsertAfter,
    InsertAtLineStart,
    AppendAtLineEnd,
    OpenLineBelow,
    OpenLineAbove,
    EnterVisual,
    EnterNormal,
    // Command line (`:w`, `:q`, `:wq`, …)
    EnterCommand,
    CommandChar(char),
    CommandBackspace,
    CommandExecute,
    CommandCancel,
    // Editing
    InsertChar(char),
    /// Insert a whole string at the cursor as a single undo step (used when
    /// accepting an LSP completion).
    InsertText(String),
    InsertNewline,
    Backspace,
    DeleteUnderCursor,
    DeleteLine,
    DeleteSelection,
    // History
    Undo,
    Redo,
    // Clipboard (a single internal register)
    YankSelection,
    Paste,
    // Application
    Save,
    Quit,
    ForceQuit,
    /// Do nothing (unbound key).
    Noop,
}

/// The complete editor state for one open buffer.
pub struct Editor {
    pub buffer: Buffer,
    pub history: History,
    pub mode: Mode,
    /// The cursor (the head of the selection in visual mode).
    pub cursor: Position,
    /// The fixed end of the selection while in visual mode.
    pub anchor: Option<Position>,
    /// Preferred column, remembered across vertical motions so that moving down
    /// through a short line and back doesn't lose the original column.
    goal_column: usize,
    /// Top visible line; owned by the editor so rendering and motions agree on
    /// what is on screen.
    pub scroll: usize,
    /// A transient status-line message.
    pub message: String,
    /// The text typed after `:` while in [`Mode::Command`] (without the `:`).
    pub command: String,
    /// Set when the user asks to quit.
    pub should_quit: bool,
    /// Internal yank/delete register (the "clipboard").
    register: String,
    /// The edit being accumulated into a single undo step. Consecutive typing
    /// or deleting is coalesced here and only committed to [`History`] on a
    /// boundary (leaving insert mode, an undo, or a non-contiguous edit).
    pending: Option<Edit>,
}

impl Editor {
    pub fn new(buffer: Buffer) -> Editor {
        Editor {
            buffer,
            history: History::new(),
            mode: Mode::Normal,
            cursor: Position::default(),
            anchor: None,
            goal_column: 0,
            scroll: 0,
            message: String::new(),
            command: String::new(),
            should_quit: false,
            register: String::new(),
            pending: None,
        }
    }

    /// The selected character range, if in visual mode. Used by the renderer
    /// to highlight the selection.
    pub fn selection_char_range(&self) -> Option<std::ops::Range<usize>> {
        self.anchor
            .map(|anchor| selection::selection_range(&self.buffer, anchor, self.cursor))
    }

    /// Scroll the viewport so the cursor is visible within `height` text rows.
    pub fn ensure_visible(&mut self, height: usize) {
        if height == 0 {
            return;
        }
        if self.cursor.line < self.scroll {
            self.scroll = self.cursor.line;
        } else if self.cursor.line >= self.scroll + height {
            self.scroll = self.cursor.line - height + 1;
        }
    }

    // --- small helpers ------------------------------------------------------

    /// The cursor as a flat character index.
    fn cursor_char(&self) -> usize {
        self.buffer.position_to_char(self.cursor)
    }

    /// Move the cursor to a character index and remember the new goal column.
    fn set_cursor_char(&mut self, idx: usize) {
        self.cursor = self.buffer.char_to_position(idx);
        self.goal_column = self.cursor.column;
    }

    /// The rightmost legal column on `line` for the current mode. Insert mode
    /// allows resting one past the last character; Normal/Visual do not.
    fn max_column(&self, line: usize) -> usize {
        let width = self.buffer.line_width(line);
        if self.mode.is_insert() {
            width
        } else {
            width.saturating_sub(1)
        }
    }

    /// Keep the cursor inside the buffer and inside the current line.
    fn clamp_cursor(&mut self) {
        let last_line = self.buffer.len_lines().saturating_sub(1);
        if self.cursor.line > last_line {
            self.cursor.line = last_line;
        }
        let max = self.max_column(self.cursor.line);
        if self.cursor.column > max {
            self.cursor.column = max;
        }
    }

    fn char_at(&self, idx: usize) -> Option<char> {
        self.buffer.rope().char_at(idx)
    }

    // --- history / editing --------------------------------------------------

    /// Apply an edit to the buffer, coalescing it into the pending undo step if
    /// it continues the previous one.
    fn edit(&mut self, edit: Edit) {
        let applied = self.buffer.apply(edit);
        match self.pending.take() {
            Some(prev) => match coalesce(&prev, &applied) {
                Some(merged) => self.pending = Some(merged),
                None => {
                    self.history.record(prev);
                    self.pending = Some(applied);
                }
            },
            None => self.pending = Some(applied),
        }
    }

    /// Commit the pending undo step, if any, so the next edit starts a new one.
    fn flush_history(&mut self) {
        if let Some(edit) = self.pending.take() {
            self.history.record(edit);
        }
    }

    // --- public entry point -------------------------------------------------

    /// Interpret one [`Action`]. This is the whole editor in one function.
    pub fn apply_action(&mut self, action: Action) {
        self.message.clear();
        match action {
            Action::MoveLeft => self.move_horizontal(-1),
            Action::MoveRight => self.move_horizontal(1),
            Action::MoveUp => self.move_vertical(-1),
            Action::MoveDown => self.move_vertical(1),
            Action::MoveLineStart => {
                self.cursor.column = 0;
                self.goal_column = 0;
            }
            Action::MoveLineEnd => {
                self.cursor.column = self.max_column(self.cursor.line);
                self.goal_column = self.cursor.column;
            }
            Action::MoveWordForward => {
                let idx = self.word_forward(self.cursor_char());
                self.set_cursor_char(idx);
            }
            Action::MoveWordBackward => {
                let idx = self.word_backward(self.cursor_char());
                self.set_cursor_char(idx);
            }
            Action::MoveBufferStart => self.set_cursor_char(0),
            Action::MoveBufferEnd => {
                let last = self.buffer.len_lines().saturating_sub(1);
                self.cursor = Position::new(last, 0);
                self.goal_column = 0;
            }

            Action::EnterInsert => self.set_mode(Mode::Insert),
            Action::InsertAfter => {
                self.set_mode(Mode::Insert);
                self.cursor.column =
                    (self.cursor.column + 1).min(self.max_column(self.cursor.line));
                self.goal_column = self.cursor.column;
            }
            Action::InsertAtLineStart => {
                self.set_mode(Mode::Insert);
                self.cursor.column = self.first_non_blank(self.cursor.line);
                self.goal_column = self.cursor.column;
            }
            Action::AppendAtLineEnd => {
                self.set_mode(Mode::Insert);
                self.cursor.column = self.buffer.line_width(self.cursor.line);
                self.goal_column = self.cursor.column;
            }
            Action::OpenLineBelow => self.open_line(false),
            Action::OpenLineAbove => self.open_line(true),

            Action::EnterVisual => {
                self.anchor = Some(self.cursor);
                self.set_mode(Mode::Visual);
            }
            Action::EnterNormal => self.enter_normal(),

            Action::EnterCommand => self.enter_command(),
            Action::CommandChar(c) => self.command.push(c),
            Action::CommandBackspace => {
                // Backspacing past the start of an empty command line is the
                // usual way to abandon it, like Vim.
                if self.command.pop().is_none() {
                    self.mode = Mode::Normal;
                }
            }
            Action::CommandExecute => self.execute_command(),
            Action::CommandCancel => {
                self.command.clear();
                self.mode = Mode::Normal;
            }

            Action::InsertChar(c) => self.insert_char(c),
            Action::InsertText(s) => self.insert_text(&s),
            Action::InsertNewline => self.insert_char('\n'),
            Action::Backspace => self.backspace(),
            Action::DeleteUnderCursor => self.delete_under_cursor(),
            Action::DeleteLine => self.delete_line(),
            Action::DeleteSelection => self.delete_selection(),

            Action::Undo => self.undo(),
            Action::Redo => self.redo(),

            Action::YankSelection => self.yank_selection(),
            Action::Paste => self.paste(),

            Action::Save => self.save(),
            Action::Quit => self.quit(false),
            Action::ForceQuit => self.quit(true),
            Action::Noop => {}
        }
        self.clamp_cursor();
    }

    // --- motions ------------------------------------------------------------

    fn move_horizontal(&mut self, delta: isize) {
        let col = self.cursor.column as isize + delta;
        self.cursor.column = col.max(0) as usize;
        self.clamp_cursor();
        self.goal_column = self.cursor.column;
    }

    fn move_vertical(&mut self, delta: isize) {
        let line = self.cursor.line as isize + delta;
        let last = self.buffer.len_lines().saturating_sub(1) as isize;
        self.cursor.line = line.clamp(0, last) as usize;
        // Restore the remembered goal column, clamped to this line.
        self.cursor.column = self.goal_column.min(self.max_column(self.cursor.line));
    }

    fn first_non_blank(&self, line: usize) -> usize {
        let text = self.buffer.line(line);
        text.chars()
            .take_while(|c| *c == ' ' || *c == '\t')
            .count()
            .min(self.buffer.line_width(line))
    }

    fn word_forward(&self, idx: usize) -> usize {
        let n = self.buffer.len_chars();
        let mut i = idx;
        if i >= n {
            return n;
        }
        let start = class_of(self.char_at(i).unwrap());
        if start != Class::Whitespace {
            while i < n && class_of(self.char_at(i).unwrap()) == start {
                i += 1;
            }
        }
        while i < n && class_of(self.char_at(i).unwrap()) == Class::Whitespace {
            i += 1;
        }
        i
    }

    fn word_backward(&self, idx: usize) -> usize {
        let mut i = idx;
        if i == 0 {
            return 0;
        }
        i -= 1;
        while i > 0 && class_of(self.char_at(i).unwrap()) == Class::Whitespace {
            i -= 1;
        }
        let c = class_of(self.char_at(i).unwrap());
        while i > 0 && class_of(self.char_at(i - 1).unwrap()) == c {
            i -= 1;
        }
        i
    }

    // --- mode transitions ---------------------------------------------------

    fn set_mode(&mut self, mode: Mode) {
        self.mode = mode;
    }

    fn enter_normal(&mut self) {
        // Leaving insert mode commits the typing as one undo step, and the
        // cursor steps back onto the last typed character, like Vim.
        let was_insert = self.mode.is_insert();
        self.flush_history();
        self.anchor = None;
        self.mode = Mode::Normal;
        if was_insert && self.cursor.column > 0 {
            self.cursor.column -= 1;
            self.goal_column = self.cursor.column;
        }
        self.clamp_cursor();
    }

    /// Open the `:` command line with an empty buffer.
    fn enter_command(&mut self) {
        self.flush_history();
        self.anchor = None;
        self.command.clear();
        self.mode = Mode::Command;
    }

    /// Parse and run the typed command, then return to Normal mode. Supports the
    /// familiar Vim write/quit set; anything else reports an error.
    fn execute_command(&mut self) {
        let command = std::mem::take(&mut self.command);
        self.mode = Mode::Normal;
        match command.trim() {
            "" => {}
            "w" => self.save(),
            "q" => self.quit(false),
            "q!" => self.quit(true),
            // `:wq`/`:x` save then quit; a failed save leaves the buffer
            // modified, so the plain `quit` refuses and nothing is lost.
            "wq" | "x" => {
                self.save();
                self.quit(false);
            }
            "wq!" => {
                self.save();
                self.quit(true);
            }
            other => self.message = format!("unknown command: :{other}"),
        }
    }

    // --- editing ------------------------------------------------------------

    fn insert_char(&mut self, c: char) {
        let idx = self.cursor_char();
        self.edit(Edit::insertion(idx, c.to_string()));
        self.set_cursor_char(idx + 1);
    }

    fn insert_text(&mut self, text: &str) {
        if text.is_empty() {
            return;
        }
        let idx = self.cursor_char();
        // Stand-alone undo step: commit before and after.
        self.flush_history();
        self.edit(Edit::insertion(idx, text));
        self.flush_history();
        self.set_cursor_char(idx + text.chars().count());
    }

    fn backspace(&mut self) {
        let idx = self.cursor_char();
        if idx == 0 {
            return;
        }
        self.edit(Edit::deletion(
            idx - 1,
            self.buffer.rope().slice(idx - 1..idx),
        ));
        self.set_cursor_char(idx - 1);
    }

    fn delete_under_cursor(&mut self) {
        let idx = self.cursor_char();
        // Don't delete the line's trailing newline with `x`.
        if idx >= self.buffer.len_chars() || self.char_at(idx) == Some('\n') {
            return;
        }
        self.flush_history();
        let removed = self.buffer.rope().slice(idx..idx + 1);
        self.register = removed.clone();
        self.edit(Edit::deletion(idx, removed));
        self.flush_history();
        self.clamp_cursor();
    }

    fn delete_line(&mut self) {
        let line = self.cursor.line;
        let start = self.buffer.position_to_char(Position::new(line, 0));
        let end = if line + 1 < self.buffer.len_lines() {
            self.buffer.position_to_char(Position::new(line + 1, 0))
        } else {
            self.buffer.len_chars()
        };
        if start == end {
            return;
        }
        self.flush_history();
        let removed = self.buffer.rope().slice(start..end);
        // Store with a trailing newline marker so paste knows it is line-wise.
        self.register = if removed.ends_with('\n') {
            removed.clone()
        } else {
            format!("{removed}\n")
        };
        self.edit(Edit::deletion(start, removed));
        self.flush_history();
        self.cursor = self
            .buffer
            .char_to_position(start.min(self.buffer.len_chars()));
        self.cursor.column = 0;
        self.goal_column = 0;
    }

    fn open_line(&mut self, above: bool) {
        let line = self.cursor.line;
        let at = if above {
            self.buffer.position_to_char(Position::new(line, 0))
        } else {
            self.buffer
                .position_to_char(Position::new(line, self.buffer.line_width(line)))
        };
        self.set_mode(Mode::Insert);
        self.edit(Edit::insertion(at, "\n"));
        // Cursor lands on the freshly opened blank line.
        let target = if above { at } else { at + 1 };
        self.set_cursor_char(target);
    }

    fn delete_selection(&mut self) {
        let Some(anchor) = self.anchor else { return };
        let range = selection::selection_range(&self.buffer, anchor, self.cursor);
        if range.is_empty() {
            return;
        }
        self.flush_history();
        let removed = self.buffer.rope().slice(range.clone());
        self.register = removed.clone();
        self.edit(Edit::deletion(range.start, removed));
        self.flush_history();
        self.set_cursor_char(range.start);
        self.enter_normal();
    }

    fn yank_selection(&mut self) {
        let Some(anchor) = self.anchor else { return };
        let range = selection::selection_range(&self.buffer, anchor, self.cursor);
        self.register = self.buffer.rope().slice(range.clone());
        self.set_cursor_char(range.start);
        self.enter_normal();
    }

    fn paste(&mut self) {
        if self.register.is_empty() {
            return;
        }
        self.flush_history();
        if self.register.ends_with('\n') {
            // Line-wise paste: drop the text on the line below.
            let line = self.cursor.line;
            let at = if line + 1 < self.buffer.len_lines() {
                self.buffer.position_to_char(Position::new(line + 1, 0))
            } else {
                self.buffer.len_chars()
            };
            // If pasting at the very end with no trailing newline, prefix one.
            let text = if at == self.buffer.len_chars()
                && !self.buffer.rope().to_string().ends_with('\n')
            {
                format!("\n{}", self.register.trim_end_matches('\n'))
            } else {
                self.register.clone()
            };
            let text_start = at + text.chars().take_while(|c| *c == '\n').count();
            self.edit(Edit::insertion(at, &text));
            self.set_cursor_char(text_start);
        } else {
            // Character-wise paste: after the cursor.
            let idx = self.cursor_char();
            let at = (idx + 1).min(self.buffer.len_chars());
            let reg = self.register.clone();
            self.edit(Edit::insertion(at, &reg));
            self.set_cursor_char(at + self.register.chars().count() - 1);
        }
        self.flush_history();
    }

    fn undo(&mut self) {
        self.flush_history();
        if let Some(edit) = self.history.undo() {
            let at = edit.at;
            self.buffer.apply(edit);
            self.set_cursor_char(at);
        } else {
            self.message = "Already at oldest change".to_string();
        }
        self.clamp_cursor();
    }

    fn redo(&mut self) {
        self.flush_history();
        if let Some(edit) = self.history.redo() {
            let end = edit.end();
            self.buffer.apply(edit);
            self.set_cursor_char(end);
        } else {
            self.message = "Already at newest change".to_string();
        }
        self.clamp_cursor();
    }

    fn save(&mut self) {
        match self.buffer.save() {
            Ok(()) => {
                self.message = format!("\"{}\" written", self.buffer.display_name());
            }
            Err(e) => self.message = format!("save failed: {e}"),
        }
    }

    fn quit(&mut self, force: bool) {
        if self.buffer.is_modified() && !force {
            self.message = "Unsaved changes. Save with Ctrl-S or force quit with Ctrl-Q.".to_string();
        } else {
            self.should_quit = true;
        }
    }
}

/// Try to merge two consecutive edits into one undo step. Returns `None` when
/// they are not contiguous, which forces a new undo group.
///
/// Only the cases that show up during continuous typing/deleting are handled;
/// anything else starts a fresh step, which is the conservative, correct choice.
fn coalesce(prev: &Edit, next: &Edit) -> Option<Edit> {
    let prev_insert = prev.removed.is_empty();
    let prev_delete = prev.inserted.is_empty();
    let next_insert = next.removed.is_empty();
    let next_delete = next.inserted.is_empty();

    // Typing: insert immediately after the previous insertion.
    if prev_insert && next_insert && next.at == prev.at + prev.inserted_chars() {
        return Some(Edit::insertion(
            prev.at,
            format!("{}{}", prev.inserted, next.inserted),
        ));
    }
    // Backspacing: delete immediately before the previous deletion.
    if prev_delete && next_delete && next.at + next.removed_chars() == prev.at {
        return Some(Edit::deletion(
            next.at,
            format!("{}{}", next.removed, prev.removed),
        ));
    }
    None
}

#[cfg(test)]
mod tests;
