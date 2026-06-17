//! Tests for the rope.
//!
//! Many of these check the rope against a plain `String` doing the same edits,
//! which is the simplest possible reference implementation: if the rope ever
//! disagrees with the string, the rope is wrong.

use super::Rope;

#[test]
fn empty_rope() {
    let r = Rope::new();
    assert!(r.is_empty());
    assert_eq!(r.len_chars(), 0);
    assert_eq!(r.len_lines(), 1); // an empty buffer is one empty line
    assert_eq!(r.to_string(), "");
}

#[test]
fn from_str_roundtrips() {
    for s in ["", "a", "hello", "line1\nline2\n", "trailing\n", "\n\n\n"] {
        assert_eq!(Rope::from_str(s).to_string(), s);
    }
}

#[test]
fn counts_chars_and_lines() {
    let r = Rope::from_str("ab\ncd\nef");
    assert_eq!(r.len_chars(), 8);
    assert_eq!(r.len_lines(), 3);

    let r = Rope::from_str("ab\ncd\n");
    assert_eq!(r.len_lines(), 3); // "ab", "cd", "" after the final newline
}

#[test]
fn insert_at_boundaries() {
    let mut r = Rope::from_str("world");
    r.insert(0, "hello ");
    assert_eq!(r.to_string(), "hello world");
    r.insert(r.len_chars(), "!");
    assert_eq!(r.to_string(), "hello world!");
    r.insert(5, ",");
    assert_eq!(r.to_string(), "hello, world!");
}

#[test]
fn remove_ranges() {
    let mut r = Rope::from_str("hello, world!");
    r.remove(5..7); // drop ", "
    assert_eq!(r.to_string(), "helloworld!");
    r.remove(0..5);
    assert_eq!(r.to_string(), "world!");
    r.remove(0..r.len_chars());
    assert_eq!(r.to_string(), "");
}

#[test]
fn char_and_slice() {
    let r = Rope::from_str("abcdef");
    assert_eq!(r.char_at(0), Some('a'));
    assert_eq!(r.char_at(5), Some('f'));
    assert_eq!(r.char_at(6), None);
    assert_eq!(r.slice(1..4), "bcd");
    assert_eq!(r.slice(0..r.len_chars()), "abcdef");
    assert_eq!(r.slice(4..100), "ef"); // clamps
}

#[test]
fn line_navigation() {
    let r = Rope::from_str("first\nsecond\nthird");
    assert_eq!(r.line_to_char(0), 0);
    assert_eq!(r.line_to_char(1), 6);
    assert_eq!(r.line_to_char(2), 13);
    assert_eq!(r.line_to_char(3), r.len_chars()); // past the last line

    assert_eq!(r.char_to_line(0), 0);
    assert_eq!(r.char_to_line(5), 0); // the newline char itself is on line 0
    assert_eq!(r.char_to_line(6), 1);
    assert_eq!(r.char_to_line(13), 2);

    assert_eq!(r.line(0), "first\n");
    assert_eq!(r.line(1), "second\n");
    assert_eq!(r.line(2), "third");
    assert_eq!(r.line(3), "");

    assert_eq!(r.line_len(0), 6);
    assert_eq!(r.line_len(2), 5);
}

#[test]
fn unicode_is_indexed_by_char_not_byte() {
    // "héllo": é is two bytes in UTF-8, so byte and char indices diverge.
    let mut r = Rope::from_str("héllo");
    assert_eq!(r.len_chars(), 5);
    assert_eq!(r.char_at(1), Some('é'));
    r.insert(2, "X"); // between é and l, by *char* index
    assert_eq!(r.to_string(), "héXllo");
    r.remove(1..2); // drop the é
    assert_eq!(r.to_string(), "hXllo");

    // Emoji are single scalar values too.
    let r = Rope::from_str("a😀b");
    assert_eq!(r.len_chars(), 3);
    assert_eq!(r.char_at(1), Some('😀'));
    assert_eq!(r.slice(1..2), "😀");
}

#[test]
fn handles_text_larger_than_a_leaf() {
    // Force many leaves and at least one rebalance.
    let big: String = (0..10_000)
        .map(|i| if i % 80 == 79 { '\n' } else { 'x' })
        .collect();
    let mut r = Rope::from_str(&big);
    assert_eq!(r.len_chars(), big.len());
    assert_eq!(r.to_string(), big);

    // Edit deep in the middle and re-check against the reference string.
    let mut reference = big.clone();
    r.insert(5_000, "INSERTED");
    reference.insert_str(reference.char_indices().nth(5_000).unwrap().0, "INSERTED");
    assert_eq!(r.to_string(), reference);

    r.remove(100..9_000);
    let from = reference.char_indices().nth(100).unwrap().0;
    let to = reference.char_indices().nth(9_000).unwrap().0;
    reference.replace_range(from..to, "");
    assert_eq!(r.to_string(), reference);
}

/// A tiny deterministic PRNG (xorshift) so the fuzz test is reproducible
/// without pulling in the `rand` crate.
struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }
    fn below(&mut self, n: usize) -> usize {
        if n == 0 {
            0
        } else {
            (self.next() % n as u64) as usize
        }
    }
}

#[test]
fn fuzz_against_a_string() {
    let mut rng = Rng(0x9E3779B97F4A7C15);
    let mut rope = Rope::new();
    let mut reference = String::new();

    let words = ["a", "bb", "ccc", "déf", "\n", "😀", "hello world\n"];

    for _ in 0..2_000 {
        let len = reference.chars().count();
        if rng.below(3) == 0 && len > 0 {
            // Remove a random char range.
            let a = rng.below(len + 1);
            let b = rng.below(len + 1);
            let (start, end) = (a.min(b), a.max(b));
            rope.remove(start..end);
            let from = char_byte(&reference, start);
            let to = char_byte(&reference, end);
            reference.replace_range(from..to, "");
        } else {
            // Insert a random word at a random position.
            let at = rng.below(len + 1);
            let word = words[rng.below(words.len())];
            rope.insert(at, word);
            let byte = char_byte(&reference, at);
            reference.insert_str(byte, word);
        }
        assert_eq!(rope.len_chars(), reference.chars().count());
    }

    assert_eq!(rope.to_string(), reference);
    // Line metrics should agree with the reference too.
    assert_eq!(rope.len_lines(), reference.matches('\n').count() + 1);
}

/// Byte offset of character index `idx` in `s` (clamping to the end).
fn char_byte(s: &str, idx: usize) -> usize {
    s.char_indices().nth(idx).map(|(b, _)| b).unwrap_or(s.len())
}
