//! Binary entry point for the `lux` editor.
//!
//! This stays deliberately thin: it parses command-line arguments and hands
//! control to [`lux::app::run`]. All of the interesting logic lives in the
//! library so that it can be tested without a terminal.

use std::path::PathBuf;
use std::process::ExitCode;

fn main() -> ExitCode {
    let mut args = std::env::args().skip(1);

    let path = match args.next().as_deref() {
        Some("--version" | "-V") => {
            println!("lux {}", lux::VERSION);
            return ExitCode::SUCCESS;
        }
        Some("--help" | "-h") => {
            print_help();
            return ExitCode::SUCCESS;
        }
        Some(file) => Some(PathBuf::from(file)),
        None => None,
    };

    match lux::app::run(path) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("lux: {e}");
            ExitCode::FAILURE
        }
    }
}

fn print_help() {
    println!("lux {}: a modal, Helix-inspired text editor", lux::VERSION);
    println!();
    println!("USAGE:");
    println!("    lux [FILE]");
    println!();
    println!("OPTIONS:");
    println!("    -h, --help       Print this help");
    println!("    -V, --version    Print the version");
    println!();
    println!("NORMAL MODE:");
    println!("    h j k l          move left/down/up/right");
    println!("    w b              next/previous word");
    println!("    0 $              start/end of line");
    println!("    gg G             start/end of buffer");
    println!("    i a              insert before/after cursor");
    println!("    I A              insert at line start/end");
    println!("    o O              open line below/above");
    println!("    v                enter visual mode");
    println!("    x dd             delete char/line");
    println!("    u U              undo/redo");
    println!("    p                paste");
    println!("    :                command line (see below)");
    println!("    Ctrl-s           save");
    println!("    Ctrl-q Ctrl-x    quit / force quit");
    println!();
    println!("INSERT MODE:");
    println!("    <text>           type to insert");
    println!("    Esc              return to normal mode");
    println!("    Ctrl-n           request completion (Rust files)");
    println!();
    println!("COMPLETION POPUP (after Ctrl-n):");
    println!("    Ctrl-n Down Tab  next candidate");
    println!("    Ctrl-p Up        previous candidate");
    println!("    Enter            accept (inserts only the untyped part)");
    println!("    Esc              dismiss");
    println!();
    println!("VISUAL MODE:");
    println!("    motions          extend the selection");
    println!("    d x              delete selection");
    println!("    y                yank selection");
    println!("    Esc v            leave visual mode");
    println!();
    println!("COMMAND MODE (:):");
    println!("    :w               write (save)");
    println!("    :q  :q!          quit / force quit");
    println!("    :wq  :x          write and quit");
    println!();
    println!("LANGUAGE SUPPORT (Rust only):");
    println!("    Open a .rs file in a Cargo project with rust-analyzer on PATH for");
    println!("    syntax highlighting, diagnostics, and Ctrl-n completion.");
}
