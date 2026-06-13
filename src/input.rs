//! Key bindings: turning a key press into an editor [`Action`].
//!
//! This is the only part of the editor that knows about specific keys. It maps
//! `(mode, key)` to an [`Action`]; the editor then interprets the action. A
//! tiny amount of state is needed for multi-key commands like `dd` (delete
//! line) and `gg` (go to top), held in [`Input::pending`].

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::editor::{Action, Mode};

/// Translates key events into actions, remembering a one-key "leader" so that
/// sequences such as `dd` and `gg` work.
#[derive(Default)]
pub struct Input {
    /// A pending leader key (`d` or `g`) awaiting its second key.
    pending: Option<char>,
}

impl Input {
    pub fn new() -> Input {
        Input { pending: None }
    }

    /// Resolve a key press in the given mode into an [`Action`].
    pub fn resolve(&mut self, mode: Mode, key: KeyEvent) -> Action {
        match mode {
            Mode::Insert => self.resolve_insert(key),
            Mode::Normal | Mode::Visual => self.resolve_normal_like(mode, key),
        }
    }

    fn resolve_insert(&mut self, key: KeyEvent) -> Action {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        match key.code {
            KeyCode::Esc => Action::EnterNormal,
            KeyCode::Enter => Action::InsertNewline,
            KeyCode::Backspace => Action::Backspace,
            KeyCode::Tab => Action::InsertChar('\t'),
            KeyCode::Left => Action::MoveLeft,
            KeyCode::Right => Action::MoveRight,
            KeyCode::Up => Action::MoveUp,
            KeyCode::Down => Action::MoveDown,
            KeyCode::Char('s') if ctrl => Action::Save,
            KeyCode::Char(c) if !ctrl => Action::InsertChar(c),
            _ => Action::Noop,
        }
    }

    fn resolve_normal_like(&mut self, mode: Mode, key: KeyEvent) -> Action {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);

        // Resolve a pending leader first (the second key of `dd` / `gg`).
        if let Some(leader) = self.pending.take() {
            if let KeyCode::Char(c) = key.code {
                match (leader, c) {
                    ('d', 'd') => return Action::DeleteLine,
                    ('g', 'g') => return Action::MoveBufferStart,
                    _ => {} // fall through and treat `c` as a fresh key
                }
            }
        }

        match key.code {
            // Application controls
            KeyCode::Char('s') if ctrl => Action::Save,
            KeyCode::Char('q') if ctrl => Action::Quit,
            KeyCode::Char('x') if ctrl => Action::ForceQuit,
            KeyCode::Char('r') if ctrl => Action::Redo,

            // Motions
            KeyCode::Char('h') | KeyCode::Left => Action::MoveLeft,
            KeyCode::Char('j') | KeyCode::Down => Action::MoveDown,
            KeyCode::Char('k') | KeyCode::Up => Action::MoveUp,
            KeyCode::Char('l') | KeyCode::Right => Action::MoveRight,
            KeyCode::Char('w') => Action::MoveWordForward,
            KeyCode::Char('b') => Action::MoveWordBackward,
            KeyCode::Char('0') | KeyCode::Home => Action::MoveLineStart,
            KeyCode::Char('$') | KeyCode::End => Action::MoveLineEnd,
            KeyCode::Char('G') => Action::MoveBufferEnd,

            // Leaders
            KeyCode::Char('g') => {
                self.pending = Some('g');
                Action::Noop
            }
            KeyCode::Char('d') if mode == Mode::Normal => {
                self.pending = Some('d');
                Action::Noop
            }

            // Visual-mode operators
            KeyCode::Char('d') | KeyCode::Char('x') if mode == Mode::Visual => {
                Action::DeleteSelection
            }
            KeyCode::Char('y') if mode == Mode::Visual => Action::YankSelection,

            // Mode switches / insertion
            KeyCode::Char('i') => Action::EnterInsert,
            KeyCode::Char('a') => Action::InsertAfter,
            KeyCode::Char('I') => Action::InsertAtLineStart,
            KeyCode::Char('A') => Action::AppendAtLineEnd,
            KeyCode::Char('o') => Action::OpenLineBelow,
            KeyCode::Char('O') => Action::OpenLineAbove,
            KeyCode::Char('v') => {
                if mode == Mode::Visual {
                    Action::EnterNormal
                } else {
                    Action::EnterVisual
                }
            }
            KeyCode::Esc => Action::EnterNormal,

            // Normal-mode edits
            KeyCode::Char('x') => Action::DeleteUnderCursor,
            KeyCode::Char('u') => Action::Undo,
            KeyCode::Char('p') => Action::Paste,

            _ => Action::Noop,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(c: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE)
    }
    fn ctrl(c: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL)
    }

    #[test]
    fn normal_motions() {
        let mut input = Input::new();
        assert_eq!(input.resolve(Mode::Normal, key('h')), Action::MoveLeft);
        assert_eq!(input.resolve(Mode::Normal, key('l')), Action::MoveRight);
        assert_eq!(input.resolve(Mode::Normal, key('w')), Action::MoveWordForward);
    }

    #[test]
    fn dd_requires_two_keys() {
        let mut input = Input::new();
        assert_eq!(input.resolve(Mode::Normal, key('d')), Action::Noop); // leader
        assert_eq!(input.resolve(Mode::Normal, key('d')), Action::DeleteLine);
    }

    #[test]
    fn gg_goes_to_top() {
        let mut input = Input::new();
        assert_eq!(input.resolve(Mode::Normal, key('g')), Action::Noop);
        assert_eq!(input.resolve(Mode::Normal, key('g')), Action::MoveBufferStart);
    }

    #[test]
    fn abandoned_leader_is_dropped() {
        let mut input = Input::new();
        input.resolve(Mode::Normal, key('d')); // start dd
        // A non-d key cancels the leader and is interpreted fresh.
        assert_eq!(input.resolve(Mode::Normal, key('l')), Action::MoveRight);
    }

    #[test]
    fn insert_mode_types_text() {
        let mut input = Input::new();
        assert_eq!(input.resolve(Mode::Insert, key('a')), Action::InsertChar('a'));
        assert_eq!(input.resolve(Mode::Insert, KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)), Action::EnterNormal);
    }

    #[test]
    fn d_in_visual_deletes_selection() {
        let mut input = Input::new();
        assert_eq!(input.resolve(Mode::Visual, key('d')), Action::DeleteSelection);
    }

    #[test]
    fn control_keys() {
        let mut input = Input::new();
        assert_eq!(input.resolve(Mode::Normal, ctrl('s')), Action::Save);
        assert_eq!(input.resolve(Mode::Normal, ctrl('r')), Action::Redo);
        assert_eq!(input.resolve(Mode::Normal, ctrl('q')), Action::Quit);
    }
}
