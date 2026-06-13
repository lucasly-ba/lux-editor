//! # lux
//!
//! A modal, [Helix](https://helix-editor.com/)-inspired terminal text editor,
//! written from scratch in Rust.
//!
//! The crate is organised as a small set of layered modules. Each layer only
//! depends on the layers below it, which keeps the data flow easy to follow:
//!
//! ```text
//!   main.rs              entry point: parse args, set up the terminal
//!     └── editor         the event loop and editor state machine
//!           ├── text     a Buffer (the rope + cursor + file metadata)
//!           │     └── rope   the core data structure (built from scratch)
//!           ├── history  the undo/redo *tree*
//!           ├── syntax   tree-sitter highlighting + incremental parsing
//!           ├── lsp      a from-scratch LSP client (JSON-RPC over stdio)
//!           └── ui       rendering: gutter, text, status line
//! ```
//!
//! Modules are added one at a time as the editor grows; see `JOURNEY.md` at the
//! repository root for the story of how each piece was built.

pub mod rope;
pub mod text;

/// The crate version, taken from `Cargo.toml` at compile time.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
