//! The editor's modal state.

/// Which mode the editor is in. This is the heart of "modal" editing: the same
/// key does different things depending on the mode.
///
/// - **Normal** — the default. Keys are commands (move, delete, change mode).
/// - **Insert** — keys type text. `Esc` returns to Normal.
/// - **Visual** — like Normal, but motions extend a selection that commands
///   (delete, yank) then act on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Normal,
    Insert,
    Visual,
}

impl Mode {
    /// Uppercase label for the status line (`NOR` / `INS` / `VIS`).
    pub fn short_label(self) -> &'static str {
        match self {
            Mode::Normal => "NOR",
            Mode::Insert => "INS",
            Mode::Visual => "VIS",
        }
    }

    pub fn is_insert(self) -> bool {
        matches!(self, Mode::Insert)
    }

    pub fn is_visual(self) -> bool {
        matches!(self, Mode::Visual)
    }
}
