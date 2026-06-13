<h1 align="center">lux</h1>

<p align="center">
  A modal, <a href="https://helix-editor.com/">Helix</a>-inspired terminal text editor, written from scratch in Rust.
</p>

<p align="center">
  <img src="https://img.shields.io/badge/rust-2024-orange.svg" alt="Rust 2024">
  <img src="https://img.shields.io/badge/license-MIT-blue.svg" alt="MIT">
  <img src="https://img.shields.io/badge/tests-84-brightgreen.svg" alt="84 tests">
</p>

---

**lux** is a small but real text editor. The interesting parts — the text data
structure, the syntax engine, the undo system, the language-server client — are
all built by hand rather than pulled off the shelf, so the goal of the project
is to *understand* how an editor works, not just to use one.

It is modal like Vim/Helix (normal / insert / visual), highlights code with
tree-sitter, re-parses incrementally as you type, and talks to `rust-analyzer`
over a hand-written LSP client for live diagnostics and completion.

## Demo

> A recorded demo lives at `docs/demo.gif` (record one with
> [`vhs`](https://github.com/charmbracelet/vhs) or `asciinema`). A taste of the
> UI:

```
  1 │ use std::collections::HashMap;          NOR  demo.rs
  2 │
  3 │ fn word_counts(text: &str) -> HashMap<String, usize> {
  4 │     let mut counts = HashMap::new();
  5 │     for word in text.split_whitespace() {
  6 │         *counts.entry(word).or_insert(0) += 1;
  7 E│     }   └─ E: mismatched types: expected `String`, found `&str`
  8 │ }
 ~
 ~
 INS  demo.rs [+]                                             6:38
```

## Features

Everything here is implemented from scratch unless noted:

#### Internals
- **Rope** text buffer — a balanced binary tree of text chunks, so edits are
  `O(log n)` instead of `O(n)`. Character-indexed (never byte-indexed) with
  cached line/char metrics and Fibonacci-criterion rebalancing.
- **Syntax highlighting** via [tree-sitter] — a real parser, not regex.
- **Incremental parsing** — after each edit only the changed range is re-parsed,
  reusing the rest of the syntax tree.

#### Systems
- **LSP client** — a from-scratch JSON-RPC client (including its own JSON
  parser) speaking to `rust-analyzer` for diagnostics and completion.
- **Undo/redo tree** — history is a tree, not a stack, so undoing and then typing
  never throws away a branch (like Vim's `undotree`).
- **Modal editing** — normal / insert / visual modes with Vim-style motions.

#### Polish
- Line-number gutter, visual selection, mode-aware cursor shape, status line.
- Tested (84 tests) and documented (see [`ARCHITECTURE.md`](ARCHITECTURE.md) and
  the build log in [`JOURNEY.md`](JOURNEY.md)).

[tree-sitter]: https://tree-sitter.github.io/

## Building & running

lux needs a Rust toolchain and a C compiler (tree-sitter grammars are C).

#### With Nix (recommended)

A `flake.nix` provides everything (Rust, a C toolchain, `rust-analyzer`):

```sh
nix develop            # enter the dev environment
cargo run --release -- samples/demo.rs
```

#### Without Nix

```sh
# needs: rustc/cargo (1.85+), a C compiler, and rust-analyzer on PATH for LSP
cargo run --release -- path/to/file.rs
```

Open with no argument for a scratch buffer:

```sh
cargo run --release
```

## Keys

Normal mode (Vim-like):

| Keys            | Action                          |
| --------------- | ------------------------------- |
| `h` `j` `k` `l` | move left / down / up / right   |
| `w` `b`         | next / previous word            |
| `0` `$`         | start / end of line             |
| `gg` `G`        | start / end of buffer           |
| `i` `a`         | insert before / after cursor    |
| `I` `A`         | insert at line start / end      |
| `o` `O`         | open line below / above         |
| `v`             | visual mode                     |
| `x` `dd`        | delete char / line              |
| `u` `Ctrl-r`    | undo / redo                     |
| `p`             | paste                           |
| `Ctrl-s`        | save                            |
| `Ctrl-q`        | quit (`Ctrl-x` to force)        |

Insert mode: type to insert, `Esc` to return to normal, `Ctrl-n` for completion.

Visual mode: motions extend the selection; `d`/`x` delete, `y` yank, `Esc`/`v`
to leave.

## Architecture

The code is layered so each module only depends on the ones below it:

```
editor  →  text → rope        the buffer and its data structure
        →  history            undo/redo tree
        →  syntax             tree-sitter highlighting
        →  lsp                language-server client
        →  ui                 rendering
```

One keystroke flows in a straight line: the terminal hands `app` an event,
`input` turns it into an `Action`, `editor.apply_action` mutates the buffer and
cursor, and `ui::render` paints the result. See [`ARCHITECTURE.md`](ARCHITECTURE.md)
for the module-by-module tour, and [`JOURNEY.md`](JOURNEY.md) for *why* each
piece is built the way it is.

## Testing

```sh
cargo test          # 80 unit + 3 integration (+1 ignored live)
cargo clippy        # lint clean
```

Highlights: the rope is fuzz-tested against a plain `String` reference, the undo
tree has a test proving branches survive, and the JSON/transport layers are
tested with in-memory pipes.

## Status & limitations

lux is a focused learning project, not a daily driver. Known limitations:
single buffer/window, full-document LSP sync, no search/replace or config file
yet, and the rope rebalances by rebuilding rather than rotating. These are
deliberate trade-offs to keep each subsystem small and readable.

## License

MIT — see [LICENSE](LICENSE).
