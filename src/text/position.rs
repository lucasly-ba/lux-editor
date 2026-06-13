//! A `(line, column)` coordinate into a buffer.

/// A zero-based cursor coordinate.
///
/// `line` and `column` are both counted in **characters**, never bytes, to stay
/// consistent with the rope. `column` is the number of characters from the
/// start of the line, so column 0 is the first character on the line.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default, Hash)]
pub struct Position {
    pub line: usize,
    pub column: usize,
}

impl Position {
    pub fn new(line: usize, column: usize) -> Position {
        Position { line, column }
    }
}
