//! Undo/redo as a **tree**, not a stack.
//!
//! With a plain undo stack, undoing some changes and then typing something new
//! throws away the redo branch: the work you undid is gone forever. Vim's
//! `undotree` and Helix keep the history as a *tree* instead: each change is a
//! node, and making a new change after an undo creates a *branch* rather than
//! discarding the old one. Nothing is ever lost; you can navigate back to any
//! previous state.
//!
//! ```text
//!         (root)
//!           │  type "hello"
//!         (A)
//!           │  type " world"
//!         (B)            ← undo, undo, then type " there"
//!          ╱ ╲
//!        (B)  (C)  "hello there"   both branches still reachable
//! ```
//!
//! This module only manages the *shape* of the history and hands back [`Edit`]s
//! for the caller to apply to its [`Buffer`](crate::text::Buffer); it never
//! touches the text itself. That keeps it small and easy to test.

use crate::text::Edit;

/// One state in the history tree, reached from its parent by applying `edit`.
struct Node {
    /// The edit that transforms the parent's text into this node's text.
    /// `None` only for the root, which represents the initial document.
    edit: Option<Edit>,
    parent: Option<usize>,
    /// Children in creation order; the last one is the most recent branch and
    /// is the one `redo` follows.
    children: Vec<usize>,
}

/// The undo/redo tree for a single buffer.
pub struct History {
    nodes: Vec<Node>,
    /// Index of the node whose text the buffer currently shows.
    current: usize,
}

impl History {
    /// A fresh history with a single root node (the empty/initial document).
    pub fn new() -> History {
        History {
            nodes: vec![Node {
                edit: None,
                parent: None,
                children: Vec::new(),
            }],
            current: 0,
        }
    }

    /// Record that `edit` was just applied to the buffer, creating a new node
    /// under the current one and moving onto it.
    ///
    /// If the current node already had children (because we had undone), this
    /// simply adds another branch; the existing ones stay reachable.
    pub fn record(&mut self, edit: Edit) {
        let new = self.nodes.len();
        self.nodes.push(Node {
            edit: Some(edit),
            parent: Some(self.current),
            children: Vec::new(),
        });
        self.nodes[self.current].children.push(new);
        self.current = new;
    }

    /// Step up to the parent node, returning the [`Edit`] the caller should
    /// apply to the buffer to get there (the inverse of the current node's
    /// edit). Returns `None` at the root.
    pub fn undo(&mut self) -> Option<Edit> {
        let node = &self.nodes[self.current];
        let parent = node.parent?;
        let revert = node
            .edit
            .as_ref()
            .expect("non-root node has an edit")
            .inverse();
        self.current = parent;
        Some(revert)
    }

    /// Step down to the most recent child branch, returning the [`Edit`] to
    /// re-apply. Returns `None` if there is nothing to redo.
    pub fn redo(&mut self) -> Option<Edit> {
        let child = *self.nodes[self.current].children.last()?;
        let edit = self.nodes[child]
            .edit
            .clone()
            .expect("non-root node has an edit");
        self.current = child;
        Some(edit)
    }

    /// Whether [`undo`](History::undo) would do anything.
    pub fn can_undo(&self) -> bool {
        self.nodes[self.current].parent.is_some()
    }

    /// Whether [`redo`](History::redo) would do anything.
    pub fn can_redo(&self) -> bool {
        !self.nodes[self.current].children.is_empty()
    }

    /// Total number of recorded states (including the root). Handy for a status
    /// line that wants to show how deep the history is.
    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    /// Always at least one (the root), so this is never empty; provided to keep
    /// clippy happy alongside [`len`](History::len).
    pub fn is_empty(&self) -> bool {
        false
    }
}

impl Default for History {
    fn default() -> History {
        History::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::text::Buffer;

    /// Apply an undo/redo edit to a buffer, mirroring what the editor does.
    fn step(buffer: &mut Buffer, edit: Option<Edit>) {
        if let Some(edit) = edit {
            buffer.apply(edit);
        }
    }

    #[test]
    fn linear_undo_redo() {
        let mut buf = Buffer::new();
        let mut hist = History::new();

        hist.record(buf.insert(0, "hello"));
        hist.record(buf.insert(5, " world"));
        assert_eq!(buf.rope().to_string(), "hello world");

        step(&mut buf, hist.undo());
        assert_eq!(buf.rope().to_string(), "hello");
        step(&mut buf, hist.undo());
        assert_eq!(buf.rope().to_string(), "");
        assert!(!hist.can_undo());

        step(&mut buf, hist.redo());
        assert_eq!(buf.rope().to_string(), "hello");
        step(&mut buf, hist.redo());
        assert_eq!(buf.rope().to_string(), "hello world");
        assert!(!hist.can_redo());
    }

    #[test]
    fn branching_keeps_both_paths() {
        let mut buf = Buffer::new();
        let mut hist = History::new();

        hist.record(buf.insert(0, "hello"));
        hist.record(buf.insert(5, " world")); // -> "hello world"

        // Undo back to "hello", then take a different branch.
        step(&mut buf, hist.undo());
        assert_eq!(buf.rope().to_string(), "hello");
        hist.record(buf.insert(5, " there")); // -> "hello there"
        assert_eq!(buf.rope().to_string(), "hello there");

        // redo now follows the *newest* branch (" there"), and the older
        // " world" branch is still in the tree, just not the default redo.
        step(&mut buf, hist.undo());
        assert_eq!(buf.rope().to_string(), "hello");
        step(&mut buf, hist.redo());
        assert_eq!(buf.rope().to_string(), "hello there");

        // The tree has the root + "hello" + " world" + " there" = 4 nodes,
        // proving the undone branch was not discarded.
        assert_eq!(hist.len(), 4);
    }

    #[test]
    fn nothing_to_undo_at_root() {
        let mut hist = History::new();
        assert!(!hist.can_undo());
        assert!(hist.undo().is_none());
        assert!(!hist.can_redo());
        assert!(hist.redo().is_none());
    }
}
