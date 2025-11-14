use std::fs;
use std::io::ErrorKind;

pub struct Buffer {
    pub lines: Vec<Vec<char>>,
}

impl Buffer {
    pub fn new() -> Self {
        Self { lines: Vec::new() }
    }

    pub fn from_file(path: &str) -> Self {
        let file_string = match fs::read_to_string(path) {
            Ok(content) => content,
            Err(error) => {
                if error.kind() == ErrorKind::NotFound {
                    return Self::new();
                } else {
                    panic!("{} must be a file.", path);
                }
            }
        };

        let lines = file_string
            .lines()
            .map(|line| line.chars().collect())
            .collect();
        Self { lines }
    }
}
