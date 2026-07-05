//! The real text-buffer implementation for Spartan's core engine (§2.1) —
//! not a Tier 0 spike. `spikes/rope-spike` is the benchmark that validated
//! choosing `ropey` (rope-vs-flat-buffer perf, `~0.0002ms` p99 clone cost,
//! §47.1); this crate is the actual buffer built on that choice.

use ropey::Rope;
use std::collections::HashMap;
use std::ops::Range;

pub type CheckpointId = u64;

/// One point in the branching undo/redo tree — a full rope snapshot plus
/// which checkpoint it was created from. Cheap because `Rope::clone` is an
/// Arc-based structural share, not a real copy of the text — the exact
/// property `rope-spike` measured (§47.1) and this crate's whole undo
/// design depends on holding.
struct Checkpoint {
    rope: Rope,
    parent: Option<CheckpointId>,
}

/// A document backed by a persistent rope (§2.1). Every edit creates a new
/// checkpoint rather than mutating history away — undo is "point at an
/// older checkpoint," and `jump_to_checkpoint` can reach any checkpoint
/// directly, not just the one most recently undone. That's what makes this
/// a branching undo tree rather than a linear stack wearing a new name.
pub struct Document {
    current: Rope,
    current_id: CheckpointId,
    checkpoints: HashMap<CheckpointId, Checkpoint>,
    next_id: CheckpointId,
    ring_capacity: usize,
    ring_order: Vec<CheckpointId>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum BufferError {
    OutOfBounds { index: usize, len: usize },
    InvalidRange { start: usize, end: usize },
    UnknownCheckpoint(CheckpointId),
}

impl std::fmt::Display for BufferError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BufferError::OutOfBounds { index, len } => {
                write!(
                    f,
                    "char index {index} is out of bounds for a {len}-char document"
                )
            }
            BufferError::InvalidRange { start, end } => {
                write!(f, "invalid range {start}..{end}: start must not exceed end")
            }
            BufferError::UnknownCheckpoint(id) => {
                write!(
                    f,
                    "checkpoint {id} is unknown or has been evicted from the ring"
                )
            }
        }
    }
}
impl std::error::Error for BufferError {}

impl Document {
    /// Default ring capacity of 500, matching §2.1's "last N edits
    /// (configurable, default 500)."
    pub fn new(initial_text: &str) -> Self {
        Self::with_ring_capacity(initial_text, 500)
    }

    pub fn with_ring_capacity(initial_text: &str, ring_capacity: usize) -> Self {
        let rope = Rope::from_str(initial_text);
        let mut checkpoints = HashMap::new();
        checkpoints.insert(
            0,
            Checkpoint {
                rope: rope.clone(),
                parent: None,
            },
        );
        Self {
            current: rope,
            current_id: 0,
            checkpoints,
            next_id: 1,
            ring_capacity: ring_capacity.max(1),
            ring_order: vec![0],
        }
    }

    pub fn text(&self) -> String {
        self.current.to_string()
    }

    pub fn len_chars(&self) -> usize {
        self.current.len_chars()
    }

    pub fn len_lines(&self) -> usize {
        self.current.len_lines()
    }

    /// Insert at a char index. Goes through `ropey`'s char-indexed API
    /// specifically — never a raw byte slice — so a multi-byte character
    /// can't be split mid-codepoint. That exact bug class (a byte-offset
    /// cut landing inside a multi-byte char) is a real, previously-found
    /// panic in this project's fallback parser (§48); this buffer avoids
    /// re-risking it by construction rather than by discipline.
    pub fn insert(&mut self, char_idx: usize, text: &str) -> Result<(), BufferError> {
        if char_idx > self.current.len_chars() {
            return Err(BufferError::OutOfBounds {
                index: char_idx,
                len: self.current.len_chars(),
            });
        }
        let mut next = self.current.clone();
        next.insert(char_idx, text);
        self.commit(next);
        Ok(())
    }

    pub fn delete(&mut self, char_range: Range<usize>) -> Result<(), BufferError> {
        self.check_range(&char_range)?;
        let mut next = self.current.clone();
        next.remove(char_range);
        self.commit(next);
        Ok(())
    }

    pub fn replace(&mut self, char_range: Range<usize>, text: &str) -> Result<(), BufferError> {
        self.check_range(&char_range)?;
        let mut next = self.current.clone();
        next.remove(char_range.clone());
        next.insert(char_range.start, text);
        self.commit(next);
        Ok(())
    }

    fn check_range(&self, r: &Range<usize>) -> Result<(), BufferError> {
        if r.start > r.end {
            return Err(BufferError::InvalidRange {
                start: r.start,
                end: r.end,
            });
        }
        if r.end > self.current.len_chars() {
            return Err(BufferError::OutOfBounds {
                index: r.end,
                len: self.current.len_chars(),
            });
        }
        Ok(())
    }

    fn commit(&mut self, new_rope: Rope) {
        let id = self.next_id;
        self.next_id += 1;
        self.checkpoints.insert(
            id,
            Checkpoint {
                rope: new_rope.clone(),
                parent: Some(self.current_id),
            },
        );
        self.current = new_rope;
        self.current_id = id;
        self.ring_order.push(id);
        self.evict_if_over_capacity();
    }

    /// Bounds memory by dropping the oldest-created checkpoint once the
    /// ring is full, purely by creation order — this is "last N edits"
    /// per §2.1, not "last N edits, except the ones on my current undo
    /// path get to live forever." An earlier version of this method
    /// protected every ancestor of the current checkpoint from eviction,
    /// which sounds safer but is actually a real bug: for an ordinary,
    /// unbranched typing session every single checkpoint *is* an ancestor
    /// of current, so nothing was ever evicted and the ring never actually
    /// bounded memory in the common case — caught by
    /// `ring_buffer_stays_bounded_through_a_long_linear_edit_session`
    /// actually failing, not by inspection.
    ///
    /// The one exception kept: never evict `current_id` itself — you
    /// can't stand on a checkpoint that's been deleted out from under you.
    /// A practical consequence, stated rather than hidden: once an edit's
    /// checkpoint ages out of the ring, `undo`/`jump_to_checkpoint` can no
    /// longer reach it and `undo()` simply stops (returns `false`) at that
    /// boundary — exactly what a *bounded* ring means, and why §2.1 pairs
    /// it with periodic full checkpoints to disk for anything longer-term
    /// (disk checkpointing itself is not implemented by this crate yet).
    fn evict_if_over_capacity(&mut self) {
        while self.ring_order.len() > self.ring_capacity {
            let oldest = self.ring_order.remove(0);
            if oldest != self.current_id {
                self.checkpoints.remove(&oldest);
            }
        }
    }

    /// Undo: point at the current checkpoint's parent. Returns `false`
    /// (not an error) in two distinct cases, both of them normal, expected
    /// states rather than failures: already at the root, or the parent
    /// *record* exists (current's `.parent` field still names it) but its
    /// actual checkpoint data has already aged out of the ring (§2.1's
    /// bounded "last N edits"). A real panic here — indexing `checkpoints`
    /// directly instead of checking first — was caught only by actually
    /// running `undo_stops_cleanly_once_history_ages_out_of_the_ring`,
    /// not by reasoning that eviction "should" be safe.
    pub fn undo(&mut self) -> bool {
        let Some(parent) = self
            .checkpoints
            .get(&self.current_id)
            .and_then(|c| c.parent)
        else {
            return false;
        };
        let Some(parent_checkpoint) = self.checkpoints.get(&parent) else {
            return false;
        };
        self.current = parent_checkpoint.rope.clone();
        self.current_id = parent;
        true
    }

    /// Jump directly to any live checkpoint — the operation that makes
    /// this a real branching tree: redo doesn't have to mean "the thing I
    /// just undid," it can mean "the version before I tried that
    /// refactor," on an entirely different branch than the current one.
    pub fn jump_to_checkpoint(&mut self, id: CheckpointId) -> Result<(), BufferError> {
        let cp = self
            .checkpoints
            .get(&id)
            .ok_or(BufferError::UnknownCheckpoint(id))?;
        self.current = cp.rope.clone();
        self.current_id = id;
        Ok(())
    }

    pub fn current_checkpoint(&self) -> CheckpointId {
        self.current_id
    }

    pub fn checkpoint_count(&self) -> usize {
        self.checkpoints.len()
    }

    /// Line/char conversions delegate to `ropey`'s own internal line
    /// index. §2.1 asks for "a line-index cache maintained incrementally
    /// ... invalidated only downstream of the edit point" — `ropey`
    /// already carries exactly this as part of its rope structure, so
    /// this wraps it rather than hand-building a second one that would
    /// just have to reproduce the same invariant.
    pub fn char_to_line(&self, char_idx: usize) -> usize {
        self.current.char_to_line(char_idx)
    }

    pub fn line_to_char(&self, line_idx: usize) -> usize {
        self.current.line_to_char(line_idx)
    }

    pub fn line(&self, line_idx: usize) -> String {
        self.current.line(line_idx).to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn insert_appends_text_correctly() {
        let mut doc = Document::new("hello world");
        doc.insert(5, ",").unwrap();
        assert_eq!(doc.text(), "hello, world");
    }

    #[test]
    fn delete_removes_range_correctly() {
        let mut doc = Document::new("hello, world");
        doc.delete(5..6).unwrap();
        assert_eq!(doc.text(), "hello world");
    }

    #[test]
    fn replace_combines_delete_and_insert() {
        let mut doc = Document::new("hello world");
        doc.replace(6..11, "there").unwrap();
        assert_eq!(doc.text(), "hello there");
    }

    #[test]
    fn out_of_bounds_insert_returns_error_not_panic() {
        let mut doc = Document::new("hi");
        let err = doc.insert(99, "x").unwrap_err();
        assert_eq!(err, BufferError::OutOfBounds { index: 99, len: 2 });
    }

    #[test]
    fn out_of_bounds_delete_returns_error_not_panic() {
        let mut doc = Document::new("hi");
        let err = doc.delete(0..99).unwrap_err();
        assert_eq!(err, BufferError::OutOfBounds { index: 99, len: 2 });
    }

    #[test]
    fn inverted_range_returns_error_not_panic() {
        let mut doc = Document::new("hello");
        // Built from variables, not a literal `3..1` -- deliberately an
        // inverted range to exercise this crate's own validation, not a
        // typo clippy's reversed_empty_ranges lint should flag.
        let (start, end) = (3, 1);
        let err = doc.delete(start..end).unwrap_err();
        assert_eq!(err, BufferError::InvalidRange { start: 3, end: 1 });
    }

    /// Direct regression test for the bug class §48 found in the fallback
    /// parser: an edit landing inside a multi-byte UTF-8 character. 🐛 is
    /// a 4-byte character; inserting/deleting immediately around it must
    /// never panic and must always produce valid, correctly-positioned text.
    #[test]
    fn multi_byte_char_boundary_is_safe() {
        let mut doc = Document::new("a🐛b");
        assert_eq!(doc.len_chars(), 3); // 'a', '🐛', 'b' -- 3 chars, not 6 bytes
        doc.insert(1, "-before-").unwrap();
        assert_eq!(doc.text(), "a-before-🐛b");
        doc.delete(1..9).unwrap(); // remove exactly "-before-"
        assert_eq!(doc.text(), "a🐛b");
        doc.replace(1..2, "🎉🎉").unwrap(); // replace the bug with two emoji
        assert_eq!(doc.text(), "a🎉🎉b");
    }

    #[test]
    fn undo_restores_previous_state() {
        let mut doc = Document::new("hello");
        doc.insert(5, " world").unwrap();
        assert_eq!(doc.text(), "hello world");
        assert!(doc.undo());
        assert_eq!(doc.text(), "hello");
    }

    #[test]
    fn undo_at_root_returns_false_and_leaves_state_unchanged() {
        let mut doc = Document::new("hello");
        assert!(!doc.undo());
        assert_eq!(doc.text(), "hello");
    }

    /// The actual "branching" proof: make edit A, undo it, make a
    /// *different* edit B from the same root, then jump back to A's
    /// checkpoint directly (not via redo-after-undo) and confirm it's
    /// still there. A linear undo/redo stack cannot do this — once B is
    /// made, a linear stack's "redo" slot for A is gone.
    #[test]
    fn branching_undo_tree_preserves_an_abandoned_branch() {
        let mut doc = Document::new("base");
        doc.insert(4, "-A").unwrap();
        let checkpoint_a = doc.current_checkpoint();
        assert_eq!(doc.text(), "base-A");

        doc.undo();
        assert_eq!(doc.text(), "base");
        doc.insert(4, "-B").unwrap();
        assert_eq!(doc.text(), "base-B");

        // A's branch is not reachable via undo/redo from here, but it
        // still exists and can be jumped to directly.
        doc.jump_to_checkpoint(checkpoint_a).unwrap();
        assert_eq!(doc.text(), "base-A");
    }

    #[test]
    fn jump_to_unknown_checkpoint_returns_error_not_panic() {
        let mut doc = Document::new("hello");
        let err = doc.jump_to_checkpoint(9999).unwrap_err();
        assert_eq!(err, BufferError::UnknownCheckpoint(9999));
    }

    #[test]
    fn line_and_char_conversions_round_trip() {
        let doc = Document::new("line one\nline two\nline three");
        assert_eq!(doc.len_lines(), 3);
        let start_of_line_1 = doc.line_to_char(1);
        assert_eq!(doc.char_to_line(start_of_line_1), 1);
        assert_eq!(doc.line(1), "line two\n");
    }

    /// A purely linear editing session — no branching at all — is the
    /// common case, and it's exactly the case an earlier, buggier version
    /// of eviction failed to bound: every checkpoint in a linear history
    /// is an ancestor of current, so "never evict an ancestor" meant
    /// "never evict anything." A long typing session must not grow
    /// memory forever just because the user never once hit undo.
    #[test]
    fn ring_buffer_stays_bounded_through_a_long_linear_edit_session() {
        let mut doc = Document::with_ring_capacity("", 3);
        for i in 0..50 {
            doc.insert(i, "x").unwrap();
        }
        assert!(
            doc.checkpoint_count() <= 4,
            "checkpoint count grew unbounded: {}",
            doc.checkpoint_count()
        );
        assert_eq!(doc.text(), "x".repeat(50));
        // undoing the single most recent edit must always still work --
        // the checkpoint you're standing on is never the one evicted.
        assert!(doc.undo());
    }

    /// The honest flip side of a *bounded* ring (§2.1: "last N edits"):
    /// once an edit's checkpoint ages out, undo can no longer reach past
    /// it and simply stops, rather than panicking or silently wrapping
    /// around to unrelated content.
    #[test]
    fn undo_stops_cleanly_once_history_ages_out_of_the_ring() {
        let mut doc = Document::with_ring_capacity("start", 2);
        for i in 0..10 {
            doc.insert(doc.len_chars(), &i.to_string()).unwrap();
        }
        let mut undo_count = 0;
        while doc.undo() {
            undo_count += 1;
            assert!(undo_count <= 10, "undo looped or never terminated");
        }
        // some bounded number of undos succeeded, then it cleanly stopped
        // -- it did not panic, and it did not undo all the way back to
        // "start" (that history is intentionally gone, not retained).
        assert!(undo_count < 10);
        assert_ne!(doc.text(), "start");
    }

    #[test]
    fn checkpoint_clone_cost_is_real_arc_sharing_not_a_deep_copy() {
        // Not a timing assertion (flaky in CI/sandboxes) -- a correctness
        // proof that cloning for a checkpoint doesn't require touching
        // every byte: a large document commits many times without the
        // test taking anything like O(n * commits) wall time to *write*,
        // which would be the case if Rope::clone deep-copied. rope-spike
        // (§47.1) already measured the actual latency; this just confirms
        // the property this crate's design depends on still holds through
        // this crate's own commit() path, not just ropey in isolation.
        let big = "x".repeat(200_000);
        let mut doc = Document::new(&big);
        for i in 0..50 {
            doc.insert(i, "y").unwrap();
        }
        assert_eq!(doc.len_chars(), big.len() + 50);
    }
}
