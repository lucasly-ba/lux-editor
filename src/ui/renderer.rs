//! Drawing the editor to the terminal.
//!
//! The renderer is a pure function of the editor state plus a [`View`] of
//! everything the surrounding subsystems want shown: syntax highlights, LSP
//! diagnostics and a completion popup. Given those, it paints one frame. It
//! owns no state of its own, which keeps redraws predictable.

use std::io::{self, Write};

use crossterm::cursor::{Hide, MoveTo, SetCursorStyle, Show};
use crossterm::style::{
    Color, Print, ResetColor, SetAttribute, SetBackgroundColor, SetForegroundColor,
};
use crossterm::terminal::{Clear, ClearType};
use crossterm::{QueueableCommand, queue};

use super::theme::Theme;
use crate::editor::{Editor, Mode};
use crate::text::Position;

/// Number of columns a tab expands to on screen.
const TAB_WIDTH: usize = 4;
/// Maximum rows shown in the completion popup at once.
const MENU_MAX_ROWS: usize = 8;
/// Maximum width of the completion popup.
const MENU_MAX_WIDTH: usize = 40;

/// A run of characters that should be drawn in a given colour (from syntax).
pub struct HighlightSpan {
    pub start: usize,
    pub end: usize,
    pub color: Color,
}

/// A diagnostic to mark on a line (from the LSP client).
pub struct LineDiagnostic {
    pub line: usize,
    pub tag: char,
    pub message: String,
    pub color: Color,
}

/// A completion popup: the candidate labels and which is selected.
pub struct CompletionMenu {
    pub items: Vec<String>,
    pub selected: usize,
}

/// Everything the renderer needs beyond the editor state itself.
#[derive(Default)]
pub struct View<'a> {
    pub highlights: &'a [HighlightSpan],
    pub diagnostics: &'a [LineDiagnostic],
    pub menu: Option<&'a CompletionMenu>,
}

/// Paint one frame of the editor into `out`.
pub fn render(
    out: &mut impl Write,
    editor: &Editor,
    view: &View,
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
            queue!(
                out,
                SetForegroundColor(theme.end_of_buffer),
                Print("~"),
                ResetColor
            )?;
            continue;
        }

        let marker = view
            .diagnostics
            .iter()
            .find(|d| d.line == line_idx)
            .map(|d| (d.tag, d.color));
        draw_gutter(out, line_idx, editor.cursor.line, gutter_w, marker, theme)?;
        draw_line(
            out,
            editor,
            view.highlights,
            line_idx,
            text_width,
            selection.as_ref(),
            theme,
        )?;
    }

    draw_status_line(out, editor, view, cols, rows, theme)?;

    if let Some(menu) = view.menu {
        draw_menu(out, editor, menu, gutter_w, text_rows, cols, theme)?;
    }

    position_cursor(out, editor, gutter_w, rows)?;
    out.flush()
}

/// Width reserved for the line-number gutter.
fn gutter_width(lines: usize) -> usize {
    let digits = lines.to_string().len();
    (digits + 1).max(4)
}

fn draw_gutter(
    out: &mut impl Write,
    line_idx: usize,
    cursor_line: usize,
    gutter_w: usize,
    marker: Option<(char, Color)>,
    theme: &Theme,
) -> io::Result<()> {
    if let Some((tag, color)) = marker {
        // A coloured diagnostic tag replaces the margin space.
        let number = format!("{:>width$}", line_idx + 1, width = gutter_w - 1);
        queue!(out, SetForegroundColor(theme.gutter), Print(number))?;
        queue!(out, SetForegroundColor(color), Print(tag), ResetColor)?;
        return Ok(());
    }
    let color = if line_idx == cursor_line {
        theme.gutter_current
    } else {
        theme.gutter
    };
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
    let line_start = buffer.position_to_char(Position::new(line_idx, 0));
    let text = buffer.line(line_idx);
    let text = text.strip_suffix('\n').unwrap_or(&text);

    let mut dcol = 0;
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
    view: &View,
    cols: usize,
    rows: usize,
    theme: &Theme,
) -> io::Result<()> {
    // In command mode the bottom row becomes the `:` command line instead of
    // the usual status bar.
    if editor.mode.is_command() {
        let text: String = format!(":{}", editor.command).chars().take(cols).collect();
        return queue!(
            out,
            MoveTo(0, (rows - 1) as u16),
            Clear(ClearType::CurrentLine),
            ResetColor,
            Print(text)
        );
    }

    let modified = if editor.buffer.is_modified() {
        " [+]"
    } else {
        ""
    };
    let left = format!(
        " {}  {}{}",
        editor.mode.short_label(),
        editor.buffer.display_name(),
        modified
    );

    // Prefer an explicit status message; otherwise show a diagnostic on the
    // cursor's line if there is one.
    let middle = if !editor.message.is_empty() {
        format!("  {}", editor.message)
    } else if let Some(d) = view
        .diagnostics
        .iter()
        .find(|d| d.line == editor.cursor.line)
    {
        format!("  {}: {}", d.tag, d.message)
    } else {
        String::new()
    };

    let right = format!("{}:{} ", editor.cursor.line + 1, editor.cursor.column + 1);

    let mut line = format!("{left}{middle}");
    let pad = cols.saturating_sub(line.chars().count() + right.chars().count());
    line.push_str(&" ".repeat(pad));
    line.push_str(&right);
    // Diagnostic messages from the server are often multi-line; collapse any
    // control characters to spaces so they can't break the single status row.
    let line: String = line
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .take(cols)
        .collect();

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

/// Draw the completion popup anchored below the cursor (or above if there's no
/// room below).
fn draw_menu(
    out: &mut impl Write,
    editor: &Editor,
    menu: &CompletionMenu,
    gutter_w: usize,
    text_rows: usize,
    cols: usize,
    theme: &Theme,
) -> io::Result<()> {
    if menu.items.is_empty() {
        return Ok(());
    }
    let width = menu
        .items
        .iter()
        .map(|s| s.chars().count())
        .max()
        .unwrap_or(0)
        .clamp(1, MENU_MAX_WIDTH)
        + 2;

    let cursor_x = gutter_w + display_column(&current_line(editor), editor.cursor.column);
    let cursor_y = editor.cursor.line.saturating_sub(editor.scroll);

    let rows_shown = menu.items.len().min(MENU_MAX_ROWS);
    // Below the cursor if it fits, otherwise above.
    let start_y = if cursor_y + 1 + rows_shown <= text_rows {
        cursor_y + 1
    } else {
        cursor_y.saturating_sub(rows_shown)
    };
    let start_x = cursor_x.min(cols.saturating_sub(width));

    // Scroll the visible window so the selected item is in view.
    let first = menu
        .selected
        .saturating_sub(rows_shown - 1)
        .min(menu.items.len().saturating_sub(rows_shown));

    for row in 0..rows_shown {
        let idx = first + row;
        let Some(item) = menu.items.get(idx) else {
            break;
        };
        let mut label: String = item
            .chars()
            .map(|c| if c.is_control() { ' ' } else { c })
            .take(width - 1)
            .collect();
        while label.chars().count() < width {
            label.push(' ');
        }
        let bg = if idx == menu.selected {
            theme.menu_selected_bg
        } else {
            theme.menu_bg
        };
        queue!(
            out,
            MoveTo(start_x as u16, (start_y + row) as u16),
            SetBackgroundColor(bg),
            SetForegroundColor(theme.menu_fg),
            Print(label),
            ResetColor
        )?;
    }
    Ok(())
}

fn current_line(editor: &Editor) -> String {
    let line = editor.buffer.line(editor.cursor.line);
    line.strip_suffix('\n').unwrap_or(&line).to_string()
}

fn position_cursor(
    out: &mut impl Write,
    editor: &Editor,
    gutter_w: usize,
    rows: usize,
) -> io::Result<()> {
    // On the command line the cursor sits after the typed text on the last row.
    if editor.mode.is_command() {
        let x = (1 + editor.command.chars().count()) as u16;
        let y = rows.saturating_sub(1) as u16;
        return queue!(out, SetCursorStyle::BlinkingBar, MoveTo(x, y), Show);
    }

    let dcol = display_column(&current_line(editor), editor.cursor.column);
    let x = (gutter_w + dcol) as u16;
    let y = editor.cursor.line.saturating_sub(editor.scroll) as u16;

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
            HighlightSpan {
                start: 0,
                end: 3,
                color: Color::Red,
            },
            HighlightSpan {
                start: 5,
                end: 8,
                color: Color::Blue,
            },
        ];
        assert_eq!(color_at(&spans, 1), Some(Color::Red));
        assert_eq!(color_at(&spans, 4), None);
        assert_eq!(color_at(&spans, 6), Some(Color::Blue));
    }
}
