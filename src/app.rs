//! The application runtime: terminal setup and the main event loop.
//!
//! This is the one place that talks to the real terminal. It is kept separate
//! from [`Editor`](crate::editor::Editor) so that all editing logic stays
//! testable without a TTY; this module just reads key events, feeds them to the
//! editor, and asks the renderer to paint the result.

use std::io::{self, Write};
use std::path::PathBuf;

use crossterm::cursor::{SetCursorStyle, Show};
use crossterm::event::{self, Event, KeyEventKind};
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode, size,
};
use crossterm::{ExecutableCommand, execute};

use crate::editor::Editor;
use crate::input::Input;
use crate::text::Buffer;
use crate::ui::{Theme, render};

/// Put the terminal into raw / alternate-screen mode and guarantee it is
/// restored when this value is dropped — even on a panic or an error path.
struct TerminalGuard;

impl TerminalGuard {
    fn enter() -> io::Result<TerminalGuard> {
        enable_raw_mode()?;
        io::stdout().execute(EnterAlternateScreen)?;
        Ok(TerminalGuard)
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let mut out = io::stdout();
        let _ = execute!(out, LeaveAlternateScreen, Show, SetCursorStyle::DefaultUserShape);
        let _ = disable_raw_mode();
    }
}

/// Open `path` (or a scratch buffer) and run the editor until the user quits.
pub fn run(path: Option<PathBuf>) -> io::Result<()> {
    let buffer = match &path {
        Some(p) => Buffer::from_file(p)?,
        None => Buffer::new(),
    };
    let mut editor = Editor::new(buffer);
    let mut input = Input::new();
    let theme = Theme::default();

    let _guard = TerminalGuard::enter()?;
    let mut out = io::stdout();

    loop {
        let (cols, rows) = size()?;
        let text_rows = rows.saturating_sub(1) as usize;
        editor.ensure_visible(text_rows);

        // Syntax highlighting spans are added in a later step; render plain for now.
        render(&mut out, &editor, &[], cols, rows, &theme)?;
        out.flush()?;

        if editor.should_quit {
            break;
        }

        // Block for the next event. Resize and non-press events just trigger a
        // redraw on the next loop iteration.
        if let Event::Key(key) = event::read()? {
            if matches!(key.kind, KeyEventKind::Press | KeyEventKind::Repeat) {
                let action = input.resolve(editor.mode, key);
                editor.apply_action(action);
            }
        }
    }

    Ok(())
}
