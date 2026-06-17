//! The editor's modal state.

/// Which mode the editor is in. This is the heart of "modal" editing: the same
/// key does different things depending on the mode.
///
/// - **Normal**: the default. Keys are commands (move, delete, change mode).
/// - **Insert**: keys type text. `Esc` returns to Normal.
/// - **Visual**: like Normal, but motions extend a selection that commands
///   (delete, yank) then act on.
/// - **Command**: a `:`-prefixed command line (`:w`, `:q`, `:wq`, …). Keys
///   edit the command string; `Enter` runs it, `Esc` cancels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Normal,
    Insert,
    Visual,
    Command,
}

impl Mode {
    /// Uppercase label for the status line (`NOR` / `INS` / `VIS` / `CMD`).
    pub fn short_label(self) -> &'static str {
        match self {
            Mode::Normal => "NOR",
            Mode::Insert => "INS",
            Mode::Visual => "VIS",
            Mode::Command => "CMD",
        }
    }

    pub fn is_insert(self) -> bool {
        matches!(self, Mode::Insert)
    }

    pub fn is_visual(self) -> bool {
        matches!(self, Mode::Visual)
    }

    pub fn is_command(self) -> bool {
        matches!(self, Mode::Command)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels_and_predicates() {
        assert_eq!(Mode::Normal.short_label(), "NOR");
        assert_eq!(Mode::Insert.short_label(), "INS");
        assert_eq!(Mode::Visual.short_label(), "VIS");
        assert_eq!(Mode::Command.short_label(), "CMD");

        assert!(Mode::Insert.is_insert());
        assert!(!Mode::Normal.is_insert());
        assert!(Mode::Visual.is_visual());
        assert!(!Mode::Insert.is_visual());
        assert!(Mode::Command.is_command());
        assert!(!Mode::Normal.is_command());
    }
}
