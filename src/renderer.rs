use crossterm::{
    cursor::MoveTo,
    execute,
    terminal::{Clear, ClearType},
};
use std::io::{Write, stdout};

use crate::buffer::Buffer;
use crate::cursor::Cursor;

pub fn render(buffer: &Buffer, cursor: &Cursor) {
    let mut stdout = stdout();

    // Effacer tout l'écran
    execute!(stdout, Clear(ClearType::All)).unwrap();

    // Déplacer le curseur en haut à gauche
    execute!(stdout, MoveTo(0, 0)).unwrap();

    for (y, line) in buffer.lines.iter().enumerate() {
        let mut s: String = line.iter().collect();

        // Si le curseur est sur cette ligne, insérer un symbole '|'
        if y == cursor.line {
            let col = cursor.column.min(s.len()); // ne pas dépasser la ligne
            s.insert(col, '|');
        }

        println!("{}", s);
    }

    stdout.flush().unwrap();
}
