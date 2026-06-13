//! Drawing the editor to the terminal.
//!
//! The renderer is a pure function of the editor state plus a list of syntax
//! [`HighlightSpan`]s: given those, it paints one frame. It owns no state of its
//! own, which keeps redraws predictable.

use std::io::{self, Write};

use crossterm::cursor::{Hide, MoveTo, SetCursorStyle, Show};
use crossterm::style::{
    Color, Print, ResetColor, SetAttribute, SetBackgroundColor, SetForegroundColor,
};
use crossterm::terminal::{Clear, ClearType};
use crossterm::{QueueableCommand, queue};

use super::theme::Theme;
use crate::editor::{Editor, Mode};

/// Number of columns a tab expands to on screen.
const TAB_WIDTH: usize = 4;

/// A run of characters that should be drawn in a given colour. Produced by the
/// syntax module; an empty list renders everything in the default foreground.
pub struct HighlightSpan {
    pub start: usize,
    pub end: usize,
    pub color: Color,
}

/// Paint one frame of the editor into `out`.
pub fn render(
    out: &mut impl Write,
    editor: &Editor,
    highlights: &[HighlightSpan],
    cols: u16,
    rows: u16,
    theme: &Theme,
) -> io::Result<()> {
    let cols = cols as usize;
    let rows = rows as usize;
    if cols == 0 || rows == 0 {
        return Ok(());
    }
    let text_rows = rows.saturating_sub(1); // last row is the status line
    let buffer = &editor.buffer;
    let gutter_w = gutter_width(buffer.len_lines());
    let text_width = cols.saturating_sub(gutter_w);
    let selection = editor.selection_char_range();

    out.queue(Hide)?;

    for y in 0..text_rows {
        let line_idx = editor.scroll + y;
        queue!(out, MoveTo(0, y as u16), Clear(ClearType::CurrentLine))?;

        if line_idx >= buffer.len_lines() {
            // Past the end of the buffer: a dim tilde, like Vim.
            queue!(
                out,
                SetForegroundColor(theme.end_of_buffer),
                Print("~"),
                ResetColor
            )?;
            continue;
        }

        draw_gutter(out, line_idx, editor.cursor.line, gutter_w, theme)?;
        draw_line(
            out,
            editor,
            highlights,
            line_idx,
            text_width,
            selection.as_ref(),
            theme,
        )?;
    }

    draw_status_line(out, editor, cols, rows, theme)?;
    position_cursor(out, editor, gutter_w)?;
    out.flush()
}

/// Width reserved for the line-number gutter: enough digits for the last line,
/// plus a one-space margin, at least 4 wide.
fn gutter_width(lines: usize) -> usize {
    let digits = lines.to_string().len();
    (digits + 1).max(4)
}

fn draw_gutter(
    out: &mut impl Write,
    line_idx: usize,
    cursor_line: usize,
    gutter_w: usize,
    theme: &Theme,
) -> io::Result<()> {
    let color = if line_idx == cursor_line {
        theme.gutter_current
    } else {
        theme.gutter
    };
    // Line numbers are 1-based for humans; right-aligned with a trailing space.
    let label = format!("{:>width$} ", line_idx + 1, width = gutter_w - 1);
    queue!(out, SetForegroundColor(color), Print(label), ResetColor)
}

fn draw_line(
    out: &mut impl Write,
    editor: &Editor,
    highlights: &[HighlightSpan],
    line_idx: usize,
    text_width: usize,
    selection: Option<&std::ops::Range<usize>>,
    theme: &Theme,
) -> io::Result<()> {
    let buffer = &editor.buffer;
    let line_start = buffer.position_to_char(crate::text::Position::new(line_idx, 0));
    let text = buffer.line(line_idx);
    let text = text.strip_suffix('\n').unwrap_or(&text);

    let mut dcol = 0; // display column, accounting for tab expansion
    for (ci, ch) in text.chars().enumerate() {
        if dcol >= text_width {
            break;
        }
        let char_idx = line_start + ci;
        let selected = selection.is_some_and(|r| r.contains(&char_idx));
        let fg = color_at(highlights, char_idx);

        if selected {
            out.queue(SetBackgroundColor(theme.selection_bg))?;
        }
        match fg {
            Some(c) => {
                out.queue(SetForegroundColor(c))?;
            }
            None => {
                out.queue(ResetColor)?;
                if selected {
                    out.queue(SetBackgroundColor(theme.selection_bg))?;
                }
            }
        }

        if ch == '\t' {
            let spaces = TAB_WIDTH - (dcol % TAB_WIDTH);
            for _ in 0..spaces {
                if dcol >= text_width {
                    break;
                }
                out.queue(Print(' '))?;
                dcol += 1;
            }
        } else {
            out.queue(Print(ch))?;
            dcol += 1;
        }
        out.queue(ResetColor)?;
    }
    out.queue(ResetColor)?;
    Ok(())
}

/// The colour for character `idx`, if any span covers it.
fn color_at(highlights: &[HighlightSpan], idx: usize) -> Option<Color> {
    highlights
        .iter()
        .find(|s| idx >= s.start && idx < s.end)
        .map(|s| s.color)
}

fn draw_status_line(
    out: &mut impl Write,
    editor: &Editor,
    cols: usize,
    rows: usize,
    theme: &Theme,
) -> io::Result<()> {
    let modified = if editor.buffer.is_modified() { " [+]" } else { "" };
    let left = format!(
        " {}  {}{}",
        editor.mode.short_label(),
        editor.buffer.display_name(),
        modified
    );
    let right = format!("{}:{} ", editor.cursor.line + 1, editor.cursor.column + 1);

    // The transient message (if any) sits between the file name and position.
    let middle = if editor.message.is_empty() {
        String::new()
    } else {
        format!("  {}", editor.message)
    };

    let mut line = format!("{left}{middle}");
    let pad = cols.saturating_sub(line.chars().count() + right.chars().count());
    line.push_str(&" ".repeat(pad));
    line.push_str(&right);
    // Truncate in case the message overflowed.
    let line: String = line.chars().take(cols).collect();

    queue!(
        out,
        MoveTo(0, (rows - 1) as u16),
        SetBackgroundColor(theme.status_bg),
        SetForegroundColor(theme.status_fg),
        SetAttribute(crossterm::style::Attribute::Bold),
        Print(line),
        SetAttribute(crossterm::style::Attribute::Reset),
        ResetColor
    )
}

fn position_cursor(out: &mut impl Write, editor: &Editor, gutter_w: usize) -> io::Result<()> {
    let line = editor.buffer.line(editor.cursor.line);
    let line = line.strip_suffix('\n').unwrap_or(&line);
    let dcol = display_column(line, editor.cursor.column);
    let x = (gutter_w + dcol) as u16;
    let y = editor.cursor.line.saturating_sub(editor.scroll) as u16;

    // A block cursor in normal/visual, a bar in insert — the usual modal hint.
    let style = if editor.mode == Mode::Insert {
        SetCursorStyle::BlinkingBar
    } else {
        SetCursorStyle::SteadyBlock
    };
    queue!(out, style, MoveTo(x, y), Show)
}

/// Convert a character column into a display column, expanding tabs.
fn display_column(line: &str, column: usize) -> usize {
    let mut dcol = 0;
    for ch in line.chars().take(column) {
        if ch == '\t' {
            dcol += TAB_WIDTH - (dcol % TAB_WIDTH);
        } else {
            dcol += 1;
        }
    }
    dcol
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gutter_grows_with_line_count() {
        assert_eq!(gutter_width(1), 4);
        assert_eq!(gutter_width(9999), 5);
        assert_eq!(gutter_width(100_000), 7);
    }

    #[test]
    fn tabs_expand_to_the_next_stop() {
        assert_eq!(display_column("\tx", 1), 4);
        assert_eq!(display_column("ab\t", 3), 4);
        assert_eq!(display_column("abcd\t", 5), 8);
    }

    #[test]
    fn color_lookup() {
        let spans = [
            HighlightSpan { start: 0, end: 3, color: Color::Red },
            HighlightSpan { start: 5, end: 8, color: Color::Blue },
        ];
        assert_eq!(color_at(&spans, 1), Some(Color::Red));
        assert_eq!(color_at(&spans, 4), None);
        assert_eq!(color_at(&spans, 6), Some(Color::Blue));
    }
}
