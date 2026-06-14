//! Tests for the editor state machine. None of these need a terminal: they
//! drive the editor purely through [`Action`]s and assert on the resulting
//! buffer text and cursor position.

use super::*;

fn editor(text: &str) -> Editor {
    Editor::new(Buffer::from_string(text))
}

/// Type a run of characters in insert mode.
fn type_str(ed: &mut Editor, s: &str) {
    for c in s.chars() {
        ed.apply_action(Action::InsertChar(c));
    }
}

#[test]
fn typing_inserts_at_cursor() {
    let mut ed = editor("");
    ed.apply_action(Action::EnterInsert);
    type_str(&mut ed, "hello");
    assert_eq!(ed.buffer.rope().to_string(), "hello");
    assert_eq!(ed.cursor, Position::new(0, 5));
}

#[test]
fn a_typing_run_is_one_undo_step() {
    let mut ed = editor("");
    ed.apply_action(Action::EnterInsert);
    type_str(&mut ed, "hello");
    ed.apply_action(Action::EnterNormal); // commits the typing as one step
    assert_eq!(ed.buffer.rope().to_string(), "hello");

    ed.apply_action(Action::Undo);
    assert_eq!(ed.buffer.rope().to_string(), ""); // whole word undone at once

    ed.apply_action(Action::Redo);
    assert_eq!(ed.buffer.rope().to_string(), "hello");
}

#[test]
fn horizontal_motion_clamps_in_normal_mode() {
    let mut ed = editor("abc");
    // Normal-mode cursor cannot rest past the last character.
    for _ in 0..10 {
        ed.apply_action(Action::MoveRight);
    }
    assert_eq!(ed.cursor, Position::new(0, 2));
    for _ in 0..10 {
        ed.apply_action(Action::MoveLeft);
    }
    assert_eq!(ed.cursor, Position::new(0, 0));
}

#[test]
fn vertical_motion_remembers_goal_column() {
    let mut ed = editor("longline\nx\nanother");
    ed.apply_action(Action::MoveLineEnd); // "longline" -> column 7 (last char)
    assert_eq!(ed.cursor, Position::new(0, 7));
    ed.apply_action(Action::MoveDown); // line 1 "x" is short -> clamp to col 0
    assert_eq!(ed.cursor, Position::new(1, 0));
    ed.apply_action(Action::MoveDown); // line 2 "another" -> goal column restored
    assert_eq!(ed.cursor, Position::new(2, 6)); // width 7, normal-mode max is 6
}

#[test]
fn x_deletes_character_under_cursor() {
    let mut ed = editor("hello");
    ed.apply_action(Action::MoveRight); // on 'e'
    ed.apply_action(Action::DeleteUnderCursor);
    assert_eq!(ed.buffer.rope().to_string(), "hllo");
    assert_eq!(ed.cursor, Position::new(0, 1));
}

#[test]
fn dd_deletes_the_line() {
    let mut ed = editor("one\ntwo\nthree");
    ed.apply_action(Action::MoveDown); // line 1 "two"
    ed.apply_action(Action::DeleteLine);
    assert_eq!(ed.buffer.rope().to_string(), "one\nthree");
    assert_eq!(ed.cursor.line, 1);
    // The deleted line is in the register and pastes back line-wise.
    ed.apply_action(Action::Paste);
    assert_eq!(ed.buffer.rope().to_string(), "one\nthree\ntwo");
}

#[test]
fn visual_delete_removes_selection() {
    let mut ed = editor("hello world");
    ed.apply_action(Action::EnterVisual);
    for _ in 0..4 {
        ed.apply_action(Action::MoveRight); // select "hello"
    }
    ed.apply_action(Action::DeleteSelection);
    assert_eq!(ed.buffer.rope().to_string(), " world");
    assert_eq!(ed.mode, Mode::Normal);
}

#[test]
fn visual_yank_and_paste() {
    let mut ed = editor("abcdef");
    ed.apply_action(Action::EnterVisual);
    ed.apply_action(Action::MoveRight);
    ed.apply_action(Action::MoveRight); // select "abc"
    ed.apply_action(Action::YankSelection);
    assert_eq!(ed.mode, Mode::Normal);
    // cursor back at start; paste inserts "abc" after the cursor (after 'a')
    ed.apply_action(Action::Paste);
    assert_eq!(ed.buffer.rope().to_string(), "aabcbcdef");
}

#[test]
fn open_line_below_enters_insert_on_new_line() {
    let mut ed = editor("first\nsecond");
    ed.apply_action(Action::OpenLineBelow);
    assert_eq!(ed.mode, Mode::Insert);
    assert_eq!(ed.cursor, Position::new(1, 0));
    type_str(&mut ed, "new");
    assert_eq!(ed.buffer.rope().to_string(), "first\nnew\nsecond");
}

#[test]
fn open_line_above() {
    let mut ed = editor("first\nsecond");
    ed.apply_action(Action::MoveDown); // on "second"
    ed.apply_action(Action::OpenLineAbove);
    type_str(&mut ed, "mid");
    assert_eq!(ed.buffer.rope().to_string(), "first\nmid\nsecond");
}

#[test]
fn word_motions() {
    let mut ed = editor("foo bar baz");
    ed.apply_action(Action::MoveWordForward);
    assert_eq!(ed.cursor, Position::new(0, 4)); // start of "bar"
    ed.apply_action(Action::MoveWordForward);
    assert_eq!(ed.cursor, Position::new(0, 8)); // start of "baz"
    ed.apply_action(Action::MoveWordBackward);
    assert_eq!(ed.cursor, Position::new(0, 4)); // back to "bar"
}

#[test]
fn backspace_run_is_one_undo_step() {
    let mut ed = editor("hello");
    ed.apply_action(Action::AppendAtLineEnd); // insert mode at end
    ed.apply_action(Action::Backspace);
    ed.apply_action(Action::Backspace); // delete "lo"
    ed.apply_action(Action::EnterNormal);
    assert_eq!(ed.buffer.rope().to_string(), "hel");
    ed.apply_action(Action::Undo);
    assert_eq!(ed.buffer.rope().to_string(), "hello");
}

#[test]
fn insert_text_is_one_undo_step() {
    let mut ed = editor("let x = ");
    ed.apply_action(Action::AppendAtLineEnd); // insert mode at end
    ed.apply_action(Action::InsertText("println".to_string()));
    assert_eq!(ed.buffer.rope().to_string(), "let x = println");
    ed.apply_action(Action::Undo);
    assert_eq!(ed.buffer.rope().to_string(), "let x = ");
}

#[test]
fn quit_blocks_on_unsaved_changes() {
    let mut ed = editor("x");
    ed.apply_action(Action::EnterInsert);
    type_str(&mut ed, "y");
    ed.apply_action(Action::EnterNormal);
    ed.apply_action(Action::Quit);
    assert!(!ed.should_quit); // blocked because modified
    ed.apply_action(Action::ForceQuit);
    assert!(ed.should_quit);
}

/// Type a `:` command and run it.
fn run_command(ed: &mut Editor, cmd: &str) {
    ed.apply_action(Action::EnterCommand);
    for c in cmd.chars() {
        ed.apply_action(Action::CommandChar(c));
    }
    ed.apply_action(Action::CommandExecute);
}

#[test]
fn command_line_edits_and_cancels() {
    let mut ed = editor("");
    ed.apply_action(Action::EnterCommand);
    assert_eq!(ed.mode, Mode::Command);
    ed.apply_action(Action::CommandChar('w'));
    ed.apply_action(Action::CommandChar('q'));
    assert_eq!(ed.command, "wq");
    ed.apply_action(Action::CommandCancel);
    assert_eq!(ed.mode, Mode::Normal);
    assert!(ed.command.is_empty());
}

#[test]
fn backspacing_empty_command_leaves_the_mode() {
    let mut ed = editor("");
    ed.apply_action(Action::EnterCommand);
    ed.apply_action(Action::CommandChar('q'));
    ed.apply_action(Action::CommandBackspace); // removes 'q'
    assert_eq!(ed.mode, Mode::Command);
    assert!(ed.command.is_empty());
    ed.apply_action(Action::CommandBackspace); // empty -> back to normal
    assert_eq!(ed.mode, Mode::Normal);
}

#[test]
fn colon_q_quits_only_when_clean() {
    // Clean buffer: `:q` quits.
    let mut ed = editor("hi");
    run_command(&mut ed, "q");
    assert_eq!(ed.mode, Mode::Normal);
    assert!(ed.should_quit);

    // Modified buffer: `:q` is refused, `:q!` forces it.
    let mut ed = editor("hi");
    ed.apply_action(Action::EnterInsert);
    type_str(&mut ed, "x");
    ed.apply_action(Action::EnterNormal);
    run_command(&mut ed, "q");
    assert!(!ed.should_quit);
    run_command(&mut ed, "q!");
    assert!(ed.should_quit);
}

#[test]
fn unknown_command_reports_an_error() {
    let mut ed = editor("");
    run_command(&mut ed, "nope");
    assert_eq!(ed.mode, Mode::Normal);
    assert!(ed.message.contains("unknown command"));
}
