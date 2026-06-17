# The lux build journey

This file is a developer log. It explains, in plain language, **why** each part
of lux exists and **how** it works, in the order it was built. If you are coming
back to this code after a break (or reading it for the first time), start here.

lux is a modal, Helix-inspired terminal text editor written from scratch in Rust.
The goal was to build, by hand, the pieces that real editors (Helix, Neovim,
VS Code) rely on, so that the internals are understood rather than imported:

- a **rope** instead of a flat string for the text buffer,
- **tree-sitter** syntax highlighting with **incremental** re-parsing,
- a small **LSP client** spoken over JSON-RPC,
- **undo/redo as a tree** rather than a linear stack,
- and **modal** editing (normal / insert / visual, with a `:` command line).

---

## 0. The build environment

This machine is NixOS and does not have `cargo`/`rustc` on the global `PATH`;
they live inside the dev shell described by `flake.nix`. To work on lux:

```sh
nix develop      # drops you into an environment that has cargo, rustc, etc.
cargo build
cargo test
cargo run -- src/main.rs
```

The shell is a plain `mkShell` whose package list is the whole toolchain:
`rustc`, `cargo`, `clippy`, `rustfmt`, `rust-analyzer`, and a C toolchain. That
C toolchain is there because of the very first build fix: rustc shells out to
the system `cc` to link the final binary, and tree-sitter's grammars are C
compiled at build time, so `gcc` and `pkg-config` had to be on the list before
anything would link at all. `flake.lock` pins the exact `nixpkgs` revision so
the toolchain is reproducible, and a one-line `.envrc` (`use flake`) lets
[direnv](https://direnv.net) enter the shell automatically on `cd`.

The same flake also exposes the editor as a package, so it isn't only a dev
environment: `nix build` compiles it, `nix run` launches it, and
`nix profile install` puts `lux` on your `PATH`. Since
`rust-analyzer` is an optional *runtime* dependency, the installed binary is
wrapped to carry it on its own `PATH`. LSP keeps working even when lux is
launched from outside the dev shell.

## 1. Project layout

The crate is split into a **library** (`src/lib.rs` + modules) and a thin
**binary** (`src/main.rs`). Everything interesting lives in the library so it
can be tested without spawning a terminal. The binary just parses arguments and
starts the editor.

The module layering (each layer only depends on the ones below it):

```
editor  →  text → rope        (the buffer and its data structure)
        →  history            (undo/redo tree)
        →  syntax             (tree-sitter highlighting)
        →  lsp                (language-server client)
        →  ui                 (rendering)
```

Each of the sections below is added as its own commit, so the git history reads
as a guided tour of the editor coming together.

## 2. The rope (`src/rope/`)

The first real piece is the text data structure. Storing a file as one big
`String` means every insert/delete in the middle is an `O(n)` shift of all the
bytes after the cursor. A **rope** stores the text as the leaves of a balanced
binary tree, so an edit only rewrites the `O(log n)` nodes on one root-to-leaf
path.

The implementation is deliberately small and built around two primitives:

- **`split(at)`** cuts the tree into "everything before character `at`" and
  "everything from `at` onward".
- **`concat(a, b)`** joins two trees, merging tiny neighbouring leaves so a long
  run of one-character inserts doesn't litter the tree with near-empty leaves.

Every edit is expressed with those two: `insert` is *split, then concat the new
text in the middle*; `remove` is *split twice and concat the outer pieces*.

Two design decisions worth calling out:

1. **Indexing is by `char`, never by byte.** A caller can ask to insert at
   "character 2" of `"héllo"` and never risk slicing the two-byte `é` in half.
   Branch nodes cache the character and newline counts of their subtree, which
   is what makes both character indexing and line lookup `O(log n)`.
2. **Rebalancing uses the Fibonacci criterion** from Boehm, Atkinson & Plass
   (1995): a tree of depth `d` is "balanced enough" if it holds at least
   `fib(d + 2)` characters. After an edit, if the tree fails that test it is
   rebuilt from its leaves. Cheap to check, and it keeps the tree shallow
   without rebalancing on every keystroke.

The tests (`src/rope/tests.rs`) include a 2,000-operation fuzz test that runs
the same random inserts/deletes against the rope and a plain `String` and checks
they always agree. The string is the obvious reference implementation.

## 3. The buffer (`src/text/`)

The rope only knows about characters. The **buffer** wraps it with everything
an editor needs around the text:

- the file path and a "modified since last save" flag,
- a monotonic **version** number (the LSP server needs to know which revision of
  the document a change refers to),
- file **load/save** (a missing file opens as empty but remembers its path, so
  saving creates it), and
- conversions between a flat character index and a `(line, column)`
  [`Position`], clamping out-of-range positions so the cursor can never point
  off the end of a line.

Crucially, **every mutation goes through one method**, `Buffer::apply`, and
produces an [`Edit`]: a small record of "at character `at`, this text was
replaced by that text". Because an edit carries both the old and the new text,
it is self-contained: you can undo it by applying its `inverse()` (swap the two
strings), and later subsystems (incremental parsing, LSP sync) can watch the
same stream of edits instead of diffing whole-file snapshots. This one decision
is what lets undo, syntax and LSP all stay simple.

## 4. The undo tree (`src/history/`)

Most editors implement undo as a stack: undo pops a change, redo pushes it back.
The catch is that if you undo a few changes and then type something new, the
redo stack is thrown away. The work you'd undone is gone.

lux instead keeps history as a **tree** (the same idea as Vim's `undotree` and
Helix). Each change is a node; making a new change after an undo creates a
*branch* instead of discarding the old one, so every state you've ever been in
stays reachable.

The module is intentionally narrow: it only tracks the *shape* of the history
and hands back [`Edit`]s for the editor to apply to the buffer. `undo()` returns
the *inverse* of the current node's edit and moves to the parent; `redo()`
returns the edit of the most-recently-created child and moves to it. Because the
buffer already knows how to apply any edit (and an edit knows its own inverse),
the whole undo system is a few dozen lines. The test
`branching_keeps_both_paths` is the one that shows it is really a tree: after
undoing and typing a new branch, the old branch is still present in the node
count.

## 5. Modal editing (`src/editor/`)

This is where lux becomes an editor. It is **modal**, like Vim and Helix:

- **Normal** mode: keys are commands (move around, delete, switch modes),
- **Insert** mode: keys type text, `Esc` returns to Normal,
- **Visual** mode: motions extend a selection that a command then acts on,
- **Command** mode: a `:` command line (`:w`, `:q`, `:wq`, `:q!`, `:x`).

The key design choice is that **everything the user can do is an [`Action`]**
(`MoveLeft`, `InsertChar('a')`, `DeleteLine`, `Undo`, …) and there is exactly
one function that interprets them, `Editor::apply_action`. The input layer's only
job is to turn a keypress into an action for the current mode; all the actual
editing logic lives behind that one entry point, with no terminal in sight. That
is why the editor has a thorough test suite that never opens a terminal. It
just feeds actions in and checks the text and cursor that come out.

Details worth highlighting:

- **Cursor clamping is mode-aware.** In Normal/Visual the cursor cannot rest
  past the last character of a line; in Insert it can sit one past the end so
  you can append. A remembered *goal column* means moving down through a short
  line and back doesn't lose your place.
- **Undo grouping.** Typing a whole word should undo in one step, not one
  keystroke at a time. The editor accumulates contiguous inserts (and contiguous
  backspaces) into a single *pending* edit and only commits it to the history
  tree on a boundary: leaving insert mode, an undo, or a non-contiguous edit
  (`coalesce`). The tests `a_typing_run_is_one_undo_step` and
  `backspace_run_is_one_undo_step` pin this behaviour down.
- **Word motions** classify characters into word / punctuation / whitespace, the
  same rule Vim's `w`/`b` use.
- **The `:` command line.** Pressing `:` switches to Command mode, where keys
  build a string shown on the bottom row instead of editing the buffer; `Enter`
  runs it, `Esc` cancels, and backspacing past the start drops back to Normal.
  This stays faithful to the one-entry-point design: the command keys are just
  more [`Action`]s (`CommandChar`, `CommandExecute`, …), and `execute_command`
  maps the parsed command onto the *same* save/quit code the `Ctrl-S`/`Ctrl-Q`
  bindings already use. `:wq` deliberately routes through the ordinary quit
  guard, so if the write fails the buffer is still marked modified and the quit
  is refused. You can't lose work to a typo'd path.

## 6. Input and the terminal (`src/input.rs`, `src/ui/`, `src/app.rs`)

Three small pieces turn the testable core into a real, running editor:

- **`input.rs`** is the *keymap*: the only code that knows about specific keys.
  It maps `(mode, key)` to an [`Action`]. The only state it keeps is a one-key
  "leader" so that two-key commands like `dd` (delete line) and `gg` (go to top)
  work.
- **`ui/`** is the *renderer*. `render()` is a pure function of the editor state
  plus a list of highlight spans: it draws the line-number gutter, the visible
  slice of text (expanding tabs, clipping long lines), the visual selection, and
  a status line showing the mode, file name, modified flag and cursor position.
  It also picks the cursor shape (a block in Normal/Visual, a bar in Insert),
  which is the usual visual hint for modal editors.
- **`app.rs`** is the *runtime*: the only module that touches the real terminal.
  It enters raw mode and the alternate screen behind a `TerminalGuard` whose
  `Drop` restores the terminal no matter how the loop exits (even on a panic),
  then runs the read → `apply_action` → `render` loop.

The data flow for one keystroke is: terminal → `app` reads an `Event` →
`input` turns it into an `Action` → `editor.apply_action` mutates the buffer and
cursor → `ui::render` paints the new state. That clean one-way flow is the whole
architecture in a sentence.

At this point lux is a usable modal editor: open a file, move around, edit,
undo/redo, select, save and quit.

## 7. Syntax highlighting (`src/syntax/`)

Highlighting is done with **tree-sitter**, the same parser generator Helix and
Neovim use, not regular expressions. tree-sitter builds a real concrete syntax
tree from the grammar (the Rust grammar is C, compiled at build time), and the
grammar ships a *highlight query* that tags nodes with names like `keyword`,
`string` or `function`. lux runs that query over the tree and maps the names to
colours (`ui/theme.rs`). Because it parses the grammar, it colours things a
regex never could: raw strings, lifetimes, generics, nested brackets.

The second, harder half is **incremental** parsing: the explicit Tier-1 goal of
"only re-parse what changed". Re-parsing the whole file on every keystroke is
wasteful, so after each edit lux:

1. diffs the new text against the previous snapshot to find the **minimal
   changed byte range** (everything between the longest common prefix and
   suffix, via `diff_input_edit`),
2. reports it to tree-sitter as an `InputEdit` via `Tree::edit`, and
3. re-parses while handing the *old* tree back in.

tree-sitter then reuses every subtree the edit didn't touch and only does work
proportional to the change. Decoupling this from the editor (the highlighter
diffs text itself rather than being handed each `Edit`) keeps the wiring trivial:
the app loop just calls `reparse` whenever the buffer version changed.

To keep highlighting cheap on large files, only the **visible** lines are
queried, and tree-sitter's byte offsets are converted to the character indices
the renderer uses. Where captures overlap, the smallest (most specific) node
wins. Tests cover the diff logic, the row/column mapping, real Rust keyword
highlighting, and an incremental edit staying consistent.

## 8. The LSP client (`src/lsp/`)

The last big piece is talking to a **language server** (`rust-analyzer`) to get
real diagnostics (the red squiggles) and completions. The Language Server
Protocol is JSON-RPC 2.0 sent over the server's stdin/stdout, and lux implements
every layer of it by hand:

1. **`json.rs`**: a from-scratch JSON value type, recursive-descent parser
   (full escapes, `\uXXXX` surrogate pairs, numbers, nesting) and serializer.
   No `serde_json`; writing it is part of showing the wire format is understood.
2. **`transport.rs`**: the message framing. Each message is a `Content-Length`
   header (counted in *bytes*, which the tests pin down with multi-byte text), a
   blank line, then the JSON body. Works over any reader/writer, so it's tested
   with in-memory buffers.
3. **`protocol.rs`**: the specific message bodies lux sends (`initialize`,
   `didOpen`, `didChange`, `completion`) and the two responses it reads
   (published diagnostics and completion items).
4. **`client.rs`**: the lifecycle. It spawns the server as a child process,
   runs a background thread that reads replies into a channel, sends requests
   with incrementing ids and matches responses back to them, stashes
   diagnostics notifications for the editor to pick up, and even answers the
   server's own requests so it never stalls. It shuts the server down on drop.

**Wiring it in** (`app.rs`): when a `.rs` file is opened, lux starts
rust-analyzer, sends `didOpen`, and on every edit sends `didChange` (and
re-parses for highlighting). Each frame it drains any published diagnostics and
the renderer marks the affected lines in the gutter and shows the message for
the cursor's line in the status bar. `Ctrl-n` in insert mode requests
completions and opens a popup; `Ctrl-n`/`Ctrl-p`/arrows navigate it and `Enter`
accepts, inserting only the part of the candidate not already typed. If
rust-analyzer isn't installed, or the file isn't Rust, lux simply runs without
any of this. The feature degrades gracefully.

In practice that makes the language features **Rust-only** today (the grammar
and the server are both Rust-specific): to see them, open a `.rs` file inside a
Cargo project (a directory with a `Cargo.toml`) with `rust-analyzer` on your
`PATH`. Give it a moment to index, then diagnostics light up the gutter and the
status bar. Completion is a *manual* trigger, not as-you-type: in insert mode
press `Ctrl-n` to ask the server for candidates, navigate the popup with
`Ctrl-n`/`Ctrl-p` (or the arrows / `Tab`), and `Enter` inserts only the part of
the candidate you haven't typed yet. Until rust-analyzer has finished indexing,
that request simply reports `no completions`. Other files still open and edit
fine, just without highlighting or LSP.

That completes the tour: a rope, a buffer, an undo tree, modal editing,
tree-sitter highlighting with incremental parsing, and a hand-written LSP
client, each its own module, each tested, wired together by a one-way data
flow from key press to repaint.

## 9. The audit (bugs the tests missed)

After the first version was "done", everything compiled, clippy was clean and
~70 tests passed, but a focused audit turned up real bugs that the unit tests
hadn't caught, mostly on the live language-server path that pure unit tests
can't exercise. Worth recording, because *passing tests are not the same as
correct*:

- **The editor froze the instant LSP started.** `poll_diagnostics` would pop a
  message, and if it wasn't a diagnostic, push it *back* onto the stash, which
  made the next pop return the same message forever. rust-analyzer floods `$/progress`
  notifications while it indexes, so this was an immediate infinite loop. The
  unit tests never fed a non-diagnostic notification, so they were green. Fix:
  drop non-diagnostic notifications instead of re-stashing them, and split out
  small `is_server_request` / `is_publish_diagnostics` classifiers that *are*
  unit-tested.
- **Diagnostic messages corrupted the status line.** rust-analyzer messages are
  often multi-line (`"mismatched types\nexpected i32, found &str"`); printing a
  raw `\n` into the one-row status line broke the display. Fix: collapse control
  characters to spaces in the status line and completion labels.
- **rust-analyzer was given the wrong root.** It was started in the file's
  directory rather than the Cargo workspace, so it couldn't find `Cargo.toml`
  and produced no diagnostics. Fix: walk up to the nearest directory containing
  `Cargo.toml` (`workspace_root`).
- **The cursor flickered when idle.** The loop repainted every 100 ms even when
  nothing changed. Fix: a `needs_redraw` flag so a frame is only drawn after a
  key, a resize, or new diagnostics.

To stop the LSP path from being a blind spot, `tests/lsp_live.rs` spawns a
*real* rust-analyzer against a throwaway crate with a type error and waits for
the diagnostic. It's `#[ignore]`d (slow, needs the binary) and run with
`cargo test --test lsp_live -- --ignored`. It both proves the happy path and
guards against the freeze regressing.

## How the tests are organised

Two layers, the standard Rust split:

- **Unit tests** live in a `#[cfg(test)] mod tests` block at the bottom of each
  source file, next to the code, and can reach private internals. Every file
  with real logic has them.
- **Integration tests** live in `tests/` and use only the public API, the way a
  real consumer would: `editing_session.rs` drives whole editing sessions, and
  `lsp_live.rs` drives a real language server.

The files with *no* tests (`lib.rs`, `main.rs`, and the `mod.rs` re-export
files) are pure glue with nothing to assert; testing them would add noise, not
safety. A few highlights worth knowing about: the rope is fuzz-tested against a
plain `String` (2,000 random ops must always agree), the undo tree has a test
proving branches survive an undo-then-edit, and the JSON/transport layers are
tested with in-memory pipes so they need no real server.

Run them with `cargo test` (fast: 87 unit + 4 integration), add
`-- --ignored` for the live LSP test, and `cargo clippy` for lints.
