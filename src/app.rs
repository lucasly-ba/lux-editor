//! The application runtime: terminal setup, the main event loop, and the wiring
//! between the editor and the optional subsystems (syntax highlighting and the
//! LSP client).
//!
//! This is the one place that talks to the real terminal. It is kept separate
//! from [`Editor`](crate::editor::Editor) so that all editing logic stays
//! testable without a TTY.

use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

use crossterm::cursor::{SetCursorStyle, Show};
use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use crossterm::style::Color;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode, size,
};
use crossterm::{ExecutableCommand, execute};

use crate::editor::{Action, Editor, Mode};
use crate::input::Input;
use crate::lsp::{Diagnostic, LspClient, Severity, protocol};
use crate::syntax::Highlighter;
use crate::text::Buffer;
use crate::ui::{CompletionMenu, LineDiagnostic, Theme, View, render};

/// How long to wait for the language server to answer a completion request.
const COMPLETION_TIMEOUT: Duration = Duration::from_millis(1500);
/// Idle poll interval, so diagnostics arriving from the server show up promptly
/// even without a keypress.
const POLL_INTERVAL: Duration = Duration::from_millis(100);

/// Put the terminal into raw / alternate-screen mode and guarantee it is
/// restored when this value is dropped, even on a panic or an error path.
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
        let _ = execute!(
            out,
            LeaveAlternateScreen,
            Show,
            SetCursorStyle::DefaultUserShape
        );
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

    // Syntax highlighting, if the file extension is recognised.
    let mut highlighter = path.as_deref().and_then(Highlighter::for_path);
    if let Some(h) = &mut highlighter {
        h.reparse(editor.buffer.rope());
    }

    // Language server, if one can be started for this file.
    let (mut lsp, lsp_uri) = start_language_server(path.as_deref(), &editor);
    let mut diagnostics: Vec<Diagnostic> = Vec::new();
    let mut menu: Option<CompletionMenu> = None;

    let _guard = TerminalGuard::enter()?;
    let mut out = io::stdout();

    // Only repaint when something changed, so an idle editor doesn't redraw (and
    // flicker the cursor) every poll interval.
    let mut needs_redraw = true;
    let mut last_size = (0u16, 0u16);

    loop {
        // Drain any diagnostics the server has published for our document.
        if let (Some(client), Some(uri)) = (&mut lsp, &lsp_uri) {
            while let Some((doc_uri, diags)) = client.poll_diagnostics() {
                if &doc_uri == uri {
                    diagnostics = diags;
                    needs_redraw = true;
                }
            }
        }

        let (cols, rows) = size()?;
        if (cols, rows) != last_size {
            last_size = (cols, rows);
            needs_redraw = true;
        }
        let text_rows = rows.saturating_sub(1) as usize;
        editor.ensure_visible(text_rows);

        if needs_redraw {
            let highlights = match &highlighter {
                Some(h) => h.spans(editor.buffer.rope(), editor.scroll, text_rows),
                None => Vec::new(),
            };
            let line_diagnostics: Vec<LineDiagnostic> = diagnostics
                .iter()
                .map(|d| LineDiagnostic {
                    line: d.line,
                    tag: d.severity.tag(),
                    message: d.message.clone(),
                    color: severity_color(d.severity),
                })
                .collect();

            let view = View {
                highlights: &highlights,
                diagnostics: &line_diagnostics,
                menu: menu.as_ref(),
            };
            render(&mut out, &editor, &view, cols, rows, &theme)?;
            out.flush()?;
            needs_redraw = false;
        }

        if editor.should_quit {
            break;
        }

        // Wait briefly for a key; on timeout, loop to refresh diagnostics.
        if !event::poll(POLL_INTERVAL)? {
            continue;
        }
        let Event::Key(key) = event::read()? else {
            continue;
        };
        if !matches!(key.kind, KeyEventKind::Press | KeyEventKind::Repeat) {
            continue;
        }
        // A key was pressed; whatever it does, the screen will need repainting.
        needs_redraw = true;
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);

        // While the completion popup is open it captures navigation keys.
        if menu.is_some() {
            match key.code {
                KeyCode::Esc => {
                    menu = None;
                    continue;
                }
                KeyCode::Down | KeyCode::Tab => {
                    menu_advance(menu.as_mut().unwrap(), 1);
                    continue;
                }
                KeyCode::Char('n') if ctrl => {
                    menu_advance(menu.as_mut().unwrap(), 1);
                    continue;
                }
                KeyCode::Up => {
                    menu_advance(menu.as_mut().unwrap(), -1);
                    continue;
                }
                KeyCode::Char('p') if ctrl => {
                    menu_advance(menu.as_mut().unwrap(), -1);
                    continue;
                }
                KeyCode::Enter => {
                    accept_completion(&mut editor, menu.take().unwrap());
                    sync_after_edit(&mut highlighter, &mut lsp, &lsp_uri, &editor);
                    continue;
                }
                // Any other key dismisses the popup and is then handled normally.
                _ => menu = None,
            }
        }

        // Ctrl-n in insert mode asks the server for completions.
        if editor.mode == Mode::Insert && ctrl && key.code == KeyCode::Char('n') {
            if let (Some(client), Some(uri)) = (&mut lsp, &lsp_uri) {
                let items = client.completion(
                    uri,
                    editor.cursor.line,
                    editor.cursor.column,
                    COMPLETION_TIMEOUT,
                );
                if items.is_empty() {
                    editor.message = "no completions".to_string();
                } else {
                    menu = Some(CompletionMenu {
                        items: items.into_iter().map(|i| i.label).collect(),
                        selected: 0,
                    });
                }
            }
            continue;
        }

        // Normal path: translate the key to an action and apply it.
        let version_before = editor.buffer.version();
        let action = input.resolve(editor.mode, key);
        editor.apply_action(action);

        if editor.buffer.version() != version_before {
            sync_after_edit(&mut highlighter, &mut lsp, &lsp_uri, &editor);
        }
    }

    Ok(())
}

/// Try to start `rust-analyzer` for `path`. Returns `(None, None)` if the file
/// isn't Rust, has no path, or the server can't be launched. In that case lux
/// just runs without LSP.
fn start_language_server(
    path: Option<&Path>,
    editor: &Editor,
) -> (Option<LspClient>, Option<String>) {
    let Some(abs) = rust_file_abspath(path) else {
        return (None, None);
    };
    // rust-analyzer needs the *workspace* root (the directory containing
    // Cargo.toml) to analyse the crate, not just the file's directory.
    let root = workspace_root(&abs);
    match LspClient::start("rust-analyzer", &root) {
        Ok(mut client) => {
            let uri = protocol::path_to_uri(&abs);
            let _ = client.did_open(
                &uri,
                "rust",
                editor.buffer.version() as i64,
                &editor.buffer.rope().to_string(),
            );
            (Some(client), Some(uri))
        }
        Err(_) => (None, None),
    }
}

/// Absolute path of `path` if it is a `.rs` file, else `None`.
fn rust_file_abspath(path: Option<&Path>) -> Option<PathBuf> {
    let path = path?;
    if path.extension().and_then(|e| e.to_str()) != Some("rs") {
        return None;
    }
    // canonicalize fails for not-yet-created files; fall back to cwd + path.
    std::fs::canonicalize(path)
        .ok()
        .or_else(|| std::env::current_dir().ok().map(|cwd| cwd.join(path)))
}

/// The nearest ancestor directory of `file` that contains a `Cargo.toml`, or the
/// file's own directory if none is found.
fn workspace_root(file: &Path) -> PathBuf {
    let mut dir = file.parent();
    while let Some(d) = dir {
        if d.join("Cargo.toml").is_file() {
            return d.to_path_buf();
        }
        dir = d.parent();
    }
    file.parent().unwrap_or(file).to_path_buf()
}

/// Re-parse for highlighting and tell the language server about the change.
fn sync_after_edit(
    highlighter: &mut Option<Highlighter>,
    lsp: &mut Option<LspClient>,
    uri: &Option<String>,
    editor: &Editor,
) {
    if let Some(h) = highlighter {
        h.reparse(editor.buffer.rope());
    }
    if let (Some(client), Some(uri)) = (lsp, uri) {
        let _ = client.did_change(
            uri,
            editor.buffer.version() as i64,
            &editor.buffer.rope().to_string(),
        );
    }
}

/// Move the menu selection by `delta`, wrapping around.
fn menu_advance(menu: &mut CompletionMenu, delta: isize) {
    let len = menu.items.len() as isize;
    if len == 0 {
        return;
    }
    menu.selected = (((menu.selected as isize + delta) % len + len) % len) as usize;
}

/// Insert the selected completion, replacing the partial word already typed.
fn accept_completion(editor: &mut Editor, menu: CompletionMenu) {
    let Some(label) = menu.items.get(menu.selected).cloned() else {
        return;
    };
    let line = editor.buffer.line(editor.cursor.line);
    let line = line.strip_suffix('\n').unwrap_or(&line);
    let prefix_len = word_prefix(line, editor.cursor.column).chars().count();
    // Replace the partial word with the full candidate. Deleting the typed
    // prefix first (rather than only appending the untyped tail) keeps the
    // result correct even when the candidate's case or shape differs from what
    // was typed. Otherwise `hashm` + `HashMap` would become `hashmHashMap`.
    for _ in 0..prefix_len {
        editor.apply_action(Action::Backspace);
    }
    editor.apply_action(Action::InsertText(label));
}

/// The identifier characters immediately before `col` on `line`.
fn word_prefix(line: &str, col: usize) -> String {
    let chars: Vec<char> = line.chars().take(col).collect();
    let start = chars
        .iter()
        .rposition(|c| !(c.is_alphanumeric() || *c == '_'))
        .map(|i| i + 1)
        .unwrap_or(0);
    chars[start..].iter().collect()
}

fn severity_color(severity: Severity) -> Color {
    match severity {
        Severity::Error => Color::Red,
        Severity::Warning => Color::Yellow,
        Severity::Information => Color::Blue,
        Severity::Hint => Color::Grey,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn word_prefix_picks_trailing_identifier() {
        assert_eq!(word_prefix("let x = vec.pus", 15), "pus");
        assert_eq!(word_prefix("    foo", 7), "foo");
        assert_eq!(word_prefix("a.b", 3), "b");
        assert_eq!(word_prefix("", 0), "");
    }

    #[test]
    fn menu_advance_wraps() {
        let mut menu = CompletionMenu {
            items: vec!["a".into(), "b".into(), "c".into()],
            selected: 0,
        };
        menu_advance(&mut menu, -1);
        assert_eq!(menu.selected, 2);
        menu_advance(&mut menu, 1);
        assert_eq!(menu.selected, 0);
    }

    #[test]
    fn accept_completion_replaces_the_typed_prefix() {
        let mut editor = Editor::new(Buffer::new());
        editor.apply_action(Action::EnterInsert);
        for c in "hashm".chars() {
            editor.apply_action(Action::InsertChar(c));
        }
        // rust-analyzer can return a candidate whose case differs from the
        // typed prefix; accepting it must not duplicate what was typed.
        let menu = CompletionMenu {
            items: vec!["HashMap".to_string()],
            selected: 0,
        };
        accept_completion(&mut editor, menu);
        assert_eq!(editor.buffer.rope().to_string(), "HashMap");
    }
}
