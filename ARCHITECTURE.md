# Architecture

This document is a map of the codebase: what each module is responsible for and
how they fit together. For the *story* of how it was built and why each design
choice was made, read [`JOURNEY.md`](JOURNEY.md).

## The big picture

lux is a **library** (`src/lib.rs` and the modules below it) wrapped in a thin
**binary** (`src/main.rs`). Putting all the logic in the library means it can be
tested without ever opening a terminal. The binary only parses arguments and
starts the runtime.

Modules are layered. Each one depends only on the layers beneath it, so there
are no cycles and the data flow is easy to follow:

```
            ┌─────────┐
            │  main   │  parse args
            └────┬────┘
            ┌────▼────┐
            │   app   │  terminal + event loop
            └────┬────┘
     ┌───────────┼───────────────┬───────────┐
┌────▼───┐  ┌────▼────┐    ┌──────▼─────┐ ┌───▼────┐
│ input  │  │ editor  │    │  syntax    │ │  lsp   │
└────────┘  └────┬────┘    └────────────┘ └────────┘
            ┌────▼────┐
            │  text   │  Buffer, Edit, Position
            └────┬────┘   + history (undo tree)
            ┌────▼────┐
            │  rope   │  the text data structure
            └─────────┘
                          ui/ renders everything
```

## One keystroke, end to end

The whole editor is a one-way data flow:

1. **`app`** blocks on the terminal and reads a key `Event`.
2. **`input`** maps `(mode, key)` to an **`Action`** (e.g. `MoveLeft`,
   `InsertChar('x')`, `DeleteLine`).
3. **`editor.apply_action`** interprets the action, mutating the **buffer** and
   **cursor** and recording the change in the **history** tree.
4. **`syntax`** and **`lsp`** are told about the change (re-parse, `didChange`).
5. **`ui::render`** paints the new state.

Because step 3 is the only place text changes, undo, incremental parsing and LSP
synchronisation can all be built by simply observing the stream of edits.

## Modules

### `rope`: the text data structure
A balanced binary tree of text chunks. Insert/delete touch only the `O(log n)`
nodes on one root-to-leaf path instead of shifting the whole file. Indexed by
character (not byte), with branch nodes caching char and newline counts for
`O(log n)` line lookups. Rebalances using the Fibonacci criterion from the
original rope paper. *Fuzz-tested against a plain `String`.*

### `text`: the buffer
`Buffer` wraps the rope with a file path, a modified flag, a monotonic version
(for the LSP), and load/save. Every mutation goes through `Buffer::apply` and
returns an `Edit` (the old and new text at a position), which knows its own
`inverse()`. `Position` converts between `(line, column)` and flat character
indices, clamping out-of-range coordinates.

### `history`: the undo tree
History is a tree of states, not a stack. `record` adds a child; `undo` returns
the inverse of the current node's edit and moves to the parent; `redo` follows
the newest child. Making a change after an undo branches rather than discarding,
so no state is ever lost.

### `editor`: the modal state machine
Owns the buffer, history, mode, cursor and selection. Every operation is an
`Action`, and `apply_action` is the single entry point that interprets them, with
no terminal involved, which is why it is thoroughly unit-tested. Handles motions,
the four modes (normal / insert / visual / command, the last being the `:`
command line for `:w`/`:q`/`:wq`), cursor clamping, a remembered goal column,
and coalescing a run of keystrokes into one undo step.

### `input`: key bindings
The only module that knows about specific keys. Maps `(mode, key)` to an
`Action`, with a one-key leader for two-key commands (`dd`, `gg`).

### `syntax`: highlighting
Uses tree-sitter to parse the code and the grammar's highlight query to colour
tokens. Re-parses **incrementally**: it diffs the new text against the previous
snapshot to find the minimal changed range, applies it to the old tree as an
`InputEdit`, and re-parses reusing untouched subtrees. Only the visible lines
are queried.

### `lsp`: the language-server client
A from-scratch JSON-RPC client in four layers: `json` (a hand-written JSON
parser/serializer), `transport` (`Content-Length` framing), `protocol` (the
message bodies and types) and `client` (spawns the server, a reader thread, and
the request/response lifecycle). Provides diagnostics and completion.

### `ui`: rendering
`render` is a pure function of the editor state plus a `View` (highlight spans,
diagnostics, completion menu). Draws the gutter, the viewport (with tab
expansion and clipping), the selection, the status line and the completion
popup. `theme` holds the colours.

### `app`: the runtime
The only module that touches the real terminal. Sets up raw mode behind a guard
that restores the terminal on any exit, runs the event loop, and wires the
optional `syntax` and `lsp` subsystems to the editor.
