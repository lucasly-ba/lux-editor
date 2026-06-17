//! End-to-end tests that drive the editor through its **public** API, the same
//! way the binary does, just without a terminal. These exercise the rope,
//! buffer, history tree and modal state machine working together.

use lux::editor::{Action, Editor, Mode};
use lux::text::{Buffer, Position};

fn type_str(ed: &mut Editor, s: &str) {
    for c in s.chars() {
        ed.apply_action(Action::InsertChar(c));
    }
}

#[test]
fn write_a_small_function_then_undo_it_all() {
    let mut ed = Editor::new(Buffer::new());

    // Type a function in insert mode.
    ed.apply_action(Action::EnterInsert);
    type_str(&mut ed, "fn add(a: i32, b: i32) -> i32 {");
    ed.apply_action(Action::InsertNewline);
    type_str(&mut ed, "    a + b");
    ed.apply_action(Action::InsertNewline);
    type_str(&mut ed, "}");
    ed.apply_action(Action::EnterNormal);

    assert_eq!(
        ed.buffer.rope().to_string(),
        "fn add(a: i32, b: i32) -> i32 {\n    a + b\n}"
    );
    assert_eq!(ed.mode, Mode::Normal);
    assert_eq!(ed.buffer.len_lines(), 3);

    // Undo unwinds the whole session (each line was one typing run + newline).
    let mut guard = 0;
    while ed.buffer.is_modified() && guard < 100 {
        ed.apply_action(Action::Undo);
        guard += 1;
    }
    assert_eq!(ed.buffer.rope().to_string(), "");
}

#[test]
fn navigate_edit_and_save_round_trip() {
    let dir = std::env::temp_dir();
    let path = dir.join(format!("lux-it-{}.txt", std::process::id()));
    let _ = std::fs::remove_file(&path);

    // Start from a file on disk.
    std::fs::write(&path, "alpha\nbeta\ngamma\n").unwrap();
    let mut ed = Editor::new(Buffer::from_file(&path).unwrap());

    // Move to line 2 ("gamma") and delete it.
    ed.apply_action(Action::MoveDown);
    ed.apply_action(Action::MoveDown);
    ed.apply_action(Action::DeleteLine);
    assert_eq!(ed.buffer.rope().to_string(), "alpha\nbeta\n");

    // Add a new last line.
    ed.apply_action(Action::MoveBufferEnd);
    ed.apply_action(Action::AppendAtLineEnd);
    type_str(&mut ed, "delta");
    ed.apply_action(Action::EnterNormal);

    ed.apply_action(Action::Save);
    assert!(!ed.buffer.is_modified());

    let on_disk = std::fs::read_to_string(&path).unwrap();
    assert_eq!(on_disk, "alpha\nbeta\ndelta");
    let _ = std::fs::remove_file(&path);
}

#[test]
fn write_and_quit_through_the_command_line() {
    let dir = std::env::temp_dir();
    let path = dir.join(format!("lux-cmd-{}.txt", std::process::id()));
    let _ = std::fs::remove_file(&path);
    std::fs::write(&path, "one\n").unwrap();

    let mut ed = Editor::new(Buffer::from_file(&path).unwrap());
    ed.apply_action(Action::AppendAtLineEnd);
    type_str(&mut ed, "!");
    ed.apply_action(Action::EnterNormal);
    assert!(ed.buffer.is_modified());

    // `:wq` saves and quits through the command line.
    ed.apply_action(Action::EnterCommand);
    for c in "wq".chars() {
        ed.apply_action(Action::CommandChar(c));
    }
    ed.apply_action(Action::CommandExecute);

    assert!(ed.should_quit);
    assert!(!ed.buffer.is_modified());
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "one!\n");
    let _ = std::fs::remove_file(&path);
}

#[test]
fn visual_selection_delete_and_paste() {
    let mut ed = Editor::new(Buffer::from_string("hello brave world"));
    // Select "brave " (the word plus the following space) and delete it.
    ed.cursor = Position::new(0, 6);
    ed.apply_action(Action::EnterVisual);
    for _ in 0..5 {
        ed.apply_action(Action::MoveRight); // head lands on the space after "brave"
    }
    ed.apply_action(Action::DeleteSelection);
    assert_eq!(ed.buffer.rope().to_string(), "hello world");

    // The deleted text is now in the register; paste it back at the end.
    ed.apply_action(Action::MoveBufferEnd);
    ed.apply_action(Action::Paste);
    assert!(ed.buffer.rope().to_string().contains("brave"));
}
