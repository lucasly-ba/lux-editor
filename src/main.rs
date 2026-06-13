//! Binary entry point for the `lux` editor.
//!
//! This stays deliberately thin: it parses command-line arguments and hands
//! control to the library. All of the interesting logic lives in `src/lib.rs`
//! and the modules beneath it so that it can be tested without a terminal.

use std::process::ExitCode;

fn main() -> ExitCode {
    let mut args = std::env::args().skip(1);

    match args.next().as_deref() {
        Some("--version" | "-V") => {
            println!("lux {}", lux::VERSION);
            ExitCode::SUCCESS
        }
        Some("--help" | "-h") => {
            print_help();
            ExitCode::SUCCESS
        }
        // A file path (or nothing) will eventually open the editor. The event
        // loop is wired up in a later step; for now we acknowledge the request
        // so the binary is runnable end to end.
        _file => {
            print_help();
            ExitCode::SUCCESS
        }
    }
}

fn print_help() {
    println!("lux {} — a modal, Helix-inspired text editor", lux::VERSION);
    println!();
    println!("USAGE:");
    println!("    lux [FILE]");
    println!();
    println!("OPTIONS:");
    println!("    -h, --help       Print this help");
    println!("    -V, --version    Print the version");
}
