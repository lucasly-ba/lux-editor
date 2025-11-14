pub struct Cursor {
    pub line: usize,
    pub column: usize,
}

impl Cursor {
    pub fn new() -> Self {
        Self { line: 0, column: 0 }
    }
}
