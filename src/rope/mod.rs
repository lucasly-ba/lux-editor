//! A **rope**: the text data structure at the heart of lux.
//!
//! A naive editor stores text as a single `String` (or a `Vec<Vec<char>>`, as
//! the first throwaway version of lux did). That makes an insert or delete in
//! the middle of a large file an `O(n)` memmove of everything after the cursor.
//! A rope instead stores the text as the leaves of a balanced binary tree, so
//! the same edit only rewrites the `O(log n)` nodes along one root-to-leaf
//! path. This is what Helix, xi and (in spirit) every serious editor use.
//!
//! ## How it works
//!
//! - **Leaves** hold a small `String` chunk (capped at [`MAX_LEAF_CHARS`]).
//! - **Branches** hold two children and cache the *summary* of their subtree:
//!   the total number of `char`s and the total number of `\n`s. Caching these
//!   summaries is what makes indexing by character or by line `O(log n)`.
//!
//! Every edit is expressed in terms of two primitives: [`Node::split`] (cut
//! the tree at a character index) and [`Node::concat`] (join two trees). This
//! keeps the logic small and easy to reason about. After an edit the tree is
//! rebalanced if it has become too lopsided; "too lopsided" is defined using
//! the Fibonacci criterion from Boehm, Atkinson & Plass (1995): a tree of depth
//! `d` is balanced only if it contains at least `fib(d + 2)` characters.
//!
//! Indexing is by **`char`** (Unicode scalar value), never by byte, so a caller
//! can never split a multi-byte UTF-8 sequence in half.

#[cfg(test)]
mod tests;

/// Maximum number of `char`s stored in a single leaf before it is split. Small
/// enough that copying a leaf during an edit is cheap, large enough that the
/// tree stays shallow.
const MAX_LEAF_CHARS: usize = 512;

/// One node of the rope tree.
///
/// Branch nodes cache `chars` and `newlines` for their *whole* subtree so that
/// navigation never has to walk into a child just to ask how big it is.
enum Node {
    Leaf {
        text: String,
        chars: usize,
        newlines: usize,
    },
    Branch {
        left: Box<Node>,
        right: Box<Node>,
        chars: usize,
        newlines: usize,
        depth: u32,
    },
}

impl Node {
    /// Build a leaf, computing and caching its metrics. `\n` is ASCII so it is
    /// safe (and faster) to count it over the raw bytes.
    fn leaf(text: String) -> Node {
        let chars = text.chars().count();
        let newlines = text.bytes().filter(|&b| b == b'\n').count();
        Node::Leaf {
            text,
            chars,
            newlines,
        }
    }

    /// Build a branch from two children, summing their cached metrics.
    fn branch(left: Node, right: Node) -> Node {
        let chars = left.chars() + right.chars();
        let newlines = left.newlines() + right.newlines();
        let depth = 1 + left.depth().max(right.depth());
        Node::Branch {
            left: Box::new(left),
            right: Box::new(right),
            chars,
            newlines,
            depth,
        }
    }

    fn chars(&self) -> usize {
        match self {
            Node::Leaf { chars, .. } | Node::Branch { chars, .. } => *chars,
        }
    }

    fn newlines(&self) -> usize {
        match self {
            Node::Leaf { newlines, .. } | Node::Branch { newlines, .. } => *newlines,
        }
    }

    fn depth(&self) -> u32 {
        match self {
            Node::Leaf { .. } => 0,
            Node::Branch { depth, .. } => *depth,
        }
    }

    /// Split this tree at character index `at`, returning the text in
    /// `0..at` and the text in `at..len` as two independent trees.
    fn split(self, at: usize) -> (Node, Node) {
        match self {
            Node::Leaf { text, .. } => {
                let byte = char_to_byte(&text, at);
                let (a, b) = text.split_at(byte);
                (Node::leaf(a.to_string()), Node::leaf(b.to_string()))
            }
            Node::Branch { left, right, .. } => {
                let left_chars = left.chars();
                match at.cmp(&left_chars) {
                    std::cmp::Ordering::Less => {
                        // The cut lands inside the left child.
                        let (ll, lr) = left.split(at);
                        (ll, Node::concat(lr, *right))
                    }
                    std::cmp::Ordering::Greater => {
                        // The cut lands inside the right child.
                        let (rl, rr) = right.split(at - left_chars);
                        (Node::concat(*left, rl), rr)
                    }
                    std::cmp::Ordering::Equal => {
                        // The cut lands exactly on the boundary between children.
                        (*left, *right)
                    }
                }
            }
        }
    }

    /// Join two trees into one. Tiny adjacent leaves are merged so that a long
    /// run of small edits does not leave the tree full of near-empty leaves.
    fn concat(left: Node, right: Node) -> Node {
        if left.chars() == 0 {
            return right;
        }
        if right.chars() == 0 {
            return left;
        }
        if let (
            Node::Leaf {
                text: lt,
                chars: lc,
                ..
            },
            Node::Leaf {
                text: rt,
                chars: rc,
                ..
            },
        ) = (&left, &right)
            && lc + rc <= MAX_LEAF_CHARS
        {
            let mut s = String::with_capacity(lt.len() + rt.len());
            s.push_str(lt);
            s.push_str(rt);
            return Node::leaf(s);
        }
        Node::branch(left, right)
    }

    /// The Boehm/Atkinson/Plass balance test: a tree of depth `d` is considered
    /// balanced only if it holds at least `fib(d + 2)` characters. A tree that
    /// fails this is degenerate enough to be worth rebuilding.
    fn is_balanced(&self) -> bool {
        self.chars() >= min_chars_for_depth(self.depth())
    }

    /// Append every leaf chunk in order into `out`. Used by rebalancing and by
    /// [`Rope::to_string`].
    fn collect_chunks<'a>(&'a self, out: &mut Vec<&'a str>) {
        match self {
            Node::Leaf { text, .. } => out.push(text),
            Node::Branch { left, right, .. } => {
                left.collect_chunks(out);
                right.collect_chunks(out);
            }
        }
    }

    /// Character index just past the `n`th (0-based) newline in this subtree.
    /// `n` must be `< self.newlines()`.
    fn char_after_nth_newline(&self, n: usize) -> usize {
        match self {
            Node::Leaf { text, .. } => {
                let mut seen = 0;
                let mut chars = 0;
                for ch in text.chars() {
                    chars += 1;
                    if ch == '\n' {
                        if seen == n {
                            return chars;
                        }
                        seen += 1;
                    }
                }
                unreachable!("char_after_nth_newline called with n >= newlines");
            }
            Node::Branch { left, right, .. } => {
                let left_nl = left.newlines();
                if n < left_nl {
                    left.char_after_nth_newline(n)
                } else {
                    left.chars() + right.char_after_nth_newline(n - left_nl)
                }
            }
        }
    }

    /// Number of newlines strictly before character index `idx`.
    fn newlines_before(&self, idx: usize) -> usize {
        match self {
            Node::Leaf { text, .. } => text.chars().take(idx).filter(|&c| c == '\n').count(),
            Node::Branch { left, right, .. } => {
                let left_chars = left.chars();
                if idx <= left_chars {
                    left.newlines_before(idx)
                } else {
                    left.newlines() + right.newlines_before(idx - left_chars)
                }
            }
        }
    }

    /// Append the characters in `start..end` of this subtree to `out`.
    fn collect_range(&self, start: usize, end: usize, out: &mut String) {
        if start >= end {
            return;
        }
        match self {
            Node::Leaf { text, .. } => {
                let from = char_to_byte(text, start);
                let to = char_to_byte(text, end);
                out.push_str(&text[from..to]);
            }
            Node::Branch { left, right, .. } => {
                let left_chars = left.chars();
                if start < left_chars {
                    left.collect_range(start, end.min(left_chars), out);
                }
                if end > left_chars {
                    let rs = start.saturating_sub(left_chars);
                    right.collect_range(rs, end - left_chars, out);
                }
            }
        }
    }

    /// The character at index `idx`, if any.
    fn char_at(&self, idx: usize) -> Option<char> {
        match self {
            Node::Leaf { text, .. } => text.chars().nth(idx),
            Node::Branch { left, right, .. } => {
                let left_chars = left.chars();
                if idx < left_chars {
                    left.char_at(idx)
                } else {
                    right.char_at(idx - left_chars)
                }
            }
        }
    }
}

/// An owned, growable rope of UTF-8 text.
///
/// `Rope` always owns a (possibly empty) root node, so every operation is total:
/// there is no "empty tree" special case to forget about.
pub struct Rope {
    root: Node,
}

impl Rope {
    /// An empty rope.
    pub fn new() -> Rope {
        Rope {
            root: Node::leaf(String::new()),
        }
    }

    /// Build a rope from a string slice, chunking it into balanced leaves.
    ///
    /// Named `from_str` to match `ropey`/`String` conventions; it is an
    /// inherent method, not the `FromStr` trait (which is fallible).
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(s: &str) -> Rope {
        let leaves = leaves_from_str(s);
        Rope {
            root: build_balanced(leaves),
        }
    }

    /// Total number of `char`s.
    pub fn len_chars(&self) -> usize {
        self.root.chars()
    }

    /// Total number of lines. A line is the text between two newlines, so a
    /// buffer always has one more line than it has newlines (an empty buffer is
    /// one empty line).
    pub fn len_lines(&self) -> usize {
        self.root.newlines() + 1
    }

    /// Whether the rope contains no characters.
    pub fn is_empty(&self) -> bool {
        self.len_chars() == 0
    }

    /// Insert `text` at character index `at`.
    ///
    /// `at` may equal [`len_chars`](Rope::len_chars) to append at the end.
    /// Panics if `at` is past the end.
    pub fn insert(&mut self, at: usize, text: &str) {
        assert!(at <= self.len_chars(), "insert index out of bounds");
        if text.is_empty() {
            return;
        }
        let root = std::mem::replace(&mut self.root, Node::leaf(String::new()));
        let (left, right) = root.split(at);
        let middle = build_balanced(leaves_from_str(text));
        self.root = Node::concat(Node::concat(left, middle), right);
        self.rebalance_if_needed();
    }

    /// Remove the characters in `range` (`start..end`, half-open).
    pub fn remove(&mut self, range: std::ops::Range<usize>) {
        let std::ops::Range { start, end } = range;
        assert!(
            start <= end && end <= self.len_chars(),
            "remove range out of bounds"
        );
        if start == end {
            return;
        }
        let root = std::mem::replace(&mut self.root, Node::leaf(String::new()));
        let (left, rest) = root.split(start);
        let (_removed, right) = rest.split(end - start);
        self.root = Node::concat(left, right);
        self.rebalance_if_needed();
    }

    /// The character at index `idx`, if in range.
    pub fn char_at(&self, idx: usize) -> Option<char> {
        if idx >= self.len_chars() {
            return None;
        }
        self.root.char_at(idx)
    }

    /// Collect the characters in `range` into a new `String`.
    pub fn slice(&self, range: std::ops::Range<usize>) -> String {
        let end = range.end.min(self.len_chars());
        let start = range.start.min(end);
        let mut out = String::new();
        self.root.collect_range(start, end, &mut out);
        out
    }

    /// Character index at which `line` (0-based) begins.
    ///
    /// `line_to_char(len_lines())` returns [`len_chars`](Rope::len_chars).
    pub fn line_to_char(&self, line: usize) -> usize {
        if line == 0 {
            return 0;
        }
        let newlines = self.root.newlines();
        if line - 1 < newlines {
            self.root.char_after_nth_newline(line - 1)
        } else {
            self.len_chars()
        }
    }

    /// The 0-based line number that character index `idx` falls on.
    pub fn char_to_line(&self, idx: usize) -> usize {
        let idx = idx.min(self.len_chars());
        self.root.newlines_before(idx)
    }

    /// The number of characters on `line`, *including* its trailing newline (if
    /// any). Returns 0 for a line past the end.
    pub fn line_len(&self, line: usize) -> usize {
        if line >= self.len_lines() {
            return 0;
        }
        self.line_to_char(line + 1) - self.line_to_char(line)
    }

    /// The text of `line` (0-based), including its trailing newline if present.
    pub fn line(&self, line: usize) -> String {
        if line >= self.len_lines() {
            return String::new();
        }
        self.slice(self.line_to_char(line)..self.line_to_char(line + 1))
    }

    /// Rebuild the tree from its leaves if it has become unbalanced.
    fn rebalance_if_needed(&mut self) {
        if !self.root.is_balanced() {
            let text = self.to_string();
            self.root = build_balanced(leaves_from_str(&text));
        }
    }
}

impl Default for Rope {
    fn default() -> Rope {
        Rope::new()
    }
}

impl std::fmt::Display for Rope {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut chunks = Vec::new();
        self.root.collect_chunks(&mut chunks);
        for chunk in chunks {
            f.write_str(chunk)?;
        }
        Ok(())
    }
}

impl From<&str> for Rope {
    fn from(s: &str) -> Rope {
        Rope::from_str(s)
    }
}

/// Convert a character index into a byte index within `text`, clamping to the
/// end of the string.
fn char_to_byte(text: &str, char_idx: usize) -> usize {
    text.char_indices()
        .nth(char_idx)
        .map(|(b, _)| b)
        .unwrap_or(text.len())
}

/// Slice `s` into a list of leaves, each at most [`MAX_LEAF_CHARS`] characters,
/// always splitting on a character boundary.
fn leaves_from_str(s: &str) -> Vec<Node> {
    if s.is_empty() {
        return vec![Node::leaf(String::new())];
    }
    let mut leaves = Vec::new();
    let mut start = 0; // byte offset of the current chunk
    let mut count = 0; // chars accumulated in the current chunk
    let mut last = 0; // byte offset just past the previous char
    for (byte, ch) in s.char_indices() {
        if count == MAX_LEAF_CHARS {
            leaves.push(Node::leaf(s[start..byte].to_string()));
            start = byte;
            count = 0;
        }
        count += 1;
        last = byte + ch.len_utf8();
    }
    if start < last {
        leaves.push(Node::leaf(s[start..].to_string()));
    }
    leaves
}

/// Combine a list of leaves into a balanced tree by repeatedly pairing
/// neighbours. `O(n)` and produces a tree of minimal depth.
fn build_balanced(mut nodes: Vec<Node>) -> Node {
    if nodes.is_empty() {
        return Node::leaf(String::new());
    }
    while nodes.len() > 1 {
        let mut next = Vec::with_capacity(nodes.len().div_ceil(2));
        let mut iter = nodes.into_iter();
        while let Some(a) = iter.next() {
            match iter.next() {
                Some(b) => next.push(Node::branch(a, b)),
                None => next.push(a),
            }
        }
        nodes = next;
    }
    nodes.pop().unwrap()
}

/// Minimum number of characters a balanced tree of `depth` may contain. This is
/// `fib(depth + 2)`, the threshold from the original rope paper.
fn min_chars_for_depth(depth: u32) -> usize {
    let mut a: usize = 1; // fib(1)
    let mut b: usize = 1; // fib(2)
    for _ in 0..depth {
        let next = a.saturating_add(b);
        a = b;
        b = next;
    }
    b
}
