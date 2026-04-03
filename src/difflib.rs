//! Token-level diffing based on the Ratcliff/Obershelp "gestalt pattern matching"
//! algorithm.
//!
//! This file is a clean-room Rust reimplementation of the algorithmic approach used by
//! Python's `difflib`, adapted to the needs of this crate. No Python source code is included.
//! Python is licensed under the Python Software Foundation License.
//!
//! The public entry point is [`compare`]. It compares two slices and returns a flat stream
//! of element-level operations using [`ChangeTag`]s.
//!
//! # Why this algorithm?
//!
//! WikiWho's authorship tracking benefits from a matcher that:
//!
//! 1. Prefers contiguous matches. The algorithm repeatedly finds the longest contiguous
//!    equal block, then recurses into the unmatched regions on the left and right. This
//!    strongly favors local structure over global token reuse.
//! 2. Downweights very common tokens. When `b` is large enough, tokens that occur too often
//!    in `b` are removed from the anchor index. That prevents frequent tokens from creating
//!    misleading matches across unrelated contexts.
//!
//! # Important behavior
//!
//! - Matching is based on contiguous runs, not longest common subsequences.
//! - The implementation is asymmetric: `b` is indexed and filtered by the autojunk
//!   heuristic, while `a` is streamed against that index. Swapping `a` and `b` can change
//!   which anchors are available.
//! - Popular elements are excluded only from the initial anchor search. Once an anchored
//!   match is found, the match is expanded outward into any adjacent equal elements,
//!   including popular ones.
//! - The final output contains only `Equal`, `Delete`, and `Insert` operations. A replaced
//!   region is represented as all deletes from `a` followed by all inserts from `b`.
//!
//! # Pipeline in this file
//!
//! 1. [`Differ::build_elem_indices`] builds an index of positions in `b` and applies the
//!    autojunk heuristic.
//! 2. [`Differ::find_longest_match`] finds the longest contiguous, non-popular anchor match
//!    inside a search window.
//! 3. [`Differ::expand_match_within_window`] extends that anchor match into adjacent equal
//!    elements inside the same window.
//! 4. [`Differ::find_matching_blocks`] splits the problem around each match until no more
//!    matches remain, yielding sorted non-overlapping equal blocks.
//! 5. [`compare`] flattens the gaps and equal blocks into per-element diff operations.

// Complexity comments use `n` as shorthand for "both input lengths are of the same order",
// which matches this crate's typical workload.

use std::{hash::Hash, ops::Range};

use rustc_hash::FxHashMap;

use crate::utils::ChangeTag;

// ---------------------------------------------------------------------------
// ClearableArray
// ---------------------------------------------------------------------------

/// A dense `usize` array that supports O(1) logical clearing via a generation counter.
///
/// Conceptually behaves like a `Vec<usize>` initialized to all zeros. Calling
/// [`Self::clear`]
/// resets every element to zero in O(1) by incrementing an internal generation counter —
/// stale entries (written in a previous generation) read as zero without being physically
/// touched. Only entries written in the current generation are "live".
///
/// This is used for the `row` buffers in [`Differ::find_longest_match`],
/// where two arrays are swapped each iteration and the incoming buffer must start clean.
/// A plain `Vec` would require O(n) zeroing per iteration (O(n²) total across the outer
/// loop); a `HashMap` avoids that but adds hashing overhead and intentions are less clear.
/// The generation counter gives direct-indexing speed with free clears.
struct ClearableArray {
    values: Vec<usize>,
    /// Each slot records the generation in which it was last written.
    /// A slot is "live" iff `generations[i] == generation`.
    generations: Vec<u32>,
    /// The current generation. Incremented by [`Self::clear`].
    generation: u32,
}

impl ClearableArray {
    /// Create an array of `len` elements, all logically zero.
    fn new(len: usize) -> Self {
        Self {
            values: vec![0; len],
            generations: vec![0; len],
            // Start at 1 so the initial state (all gens = 0) reads as "stale" → zero.
            generation: 1,
        }
    }

    /// Reset all elements to zero in O(1). Simply advances the generation counter,
    /// making every existing entry stale.
    fn clear(&mut self) {
        if let Some(new_gen) = self.generation.checked_add(1) {
            self.generation = new_gen;
        } else {
            // overflowed the generation counter, manually clear self.generations and reset generation to 1
            let len = self.values.len();

            self.generation = 1;
            self.generations = vec![0; len];
        }
    }
}

impl std::ops::Index<usize> for ClearableArray {
    type Output = usize;

    fn index(&self, i: usize) -> &usize {
        if self.generations[i] == self.generation {
            &self.values[i]
        } else {
            &0
        }
    }
}

impl std::ops::IndexMut<usize> for ClearableArray {
    fn index_mut(&mut self, i: usize) -> &mut usize {
        if self.generations[i] != self.generation {
            // Slot is stale — zero it and mark as current before returning.
            self.values[i] = 0;
            self.generations[i] = self.generation;
        }
        &mut self.values[i]
    }
}

/// Matching window or block represented as `(a_range, b_range)`.
type ABRange = (Range<usize>, Range<usize>);

/// Internal matcher state.
///
/// The implementation is intentionally asymmetric: it indexes `b`, then scans `a`
/// against that index. That asymmetry is observable because the autojunk heuristic only
/// filters elements from `b_indices`.
struct Differ<'a, T: Hash + Eq> {
    a: &'a [T],
    b: &'a [T],

    /// For each non-popular element in `b`, the sorted list of positions where it appears.
    b_indices: FxHashMap<&'a T, Vec<usize>>,
    /// DP row for the previous `i` in `find_longest_match`.
    last_row: ClearableArray,
    /// DP row currently being filled in `find_longest_match`.
    this_row: ClearableArray,
}

impl<'a, T: Hash + Eq> Differ<'a, T> {
    /// Build the asymmetric matcher and precompute the index for `b`.
    fn new(a: &'a [T], b: &'a [T]) -> Self {
        Self {
            a,
            b,
            b_indices: Self::build_elem_indices(b),
            last_row: ClearableArray::new(b.len()),
            this_row: ClearableArray::new(b.len()),
        }
    }

    /// Build the lookup table used to answer "where does this element occur in `b`?".
    ///
    /// When `list.len() >= 200`, the autojunk heuristic removes elements whose frequency is
    /// greater than `1 + list.len() / 100`. Those elements may still be part of a final
    /// equal block via [`expand_match_within_window`](Self::expand_match_within_window), but
    /// they cannot start a match on their own.
    fn build_elem_indices(list: &'a [T]) -> FxHashMap<&'a T, Vec<usize>> {
        let mut elem_indices = FxHashMap::default();

        for (i, elem) in list.iter().enumerate() {
            elem_indices
                .entry(elem)
                .and_modify(|indices: &mut Vec<usize>| indices.push(i))
                .or_insert_with(|| vec![i]);
        }

        let len = list.len();
        if len >= 200 {
            // autojunk: don't match tokens that appear more often than 1 + floor(1% * len)
            let max_count = 1 + len / 100;
            elem_indices.retain(|_, indices| indices.len() <= max_count);
        }

        elem_indices
    }

    /// Find the longest contiguous anchor match inside a search window.
    ///
    /// The returned block is the longest contiguous run shared by `self.a[a_range]` and
    /// `self.b[b_range]` that can be found by anchoring only on elements present in
    /// `b_indices`. In other words, popular elements removed by the autojunk heuristic can
    /// participate only later during [`expand_match_within_window`](Self::expand_match_within_window).
    ///
    /// `None` means that no non-empty anchor match exists in this window.
    #[doc = include_str!("difflib_find_longest_match.md")]
    fn find_longest_match(&mut self, (a_range, b_range): ABRange) -> Option<ABRange> {
        let mut longest_a_b = None;
        let mut longest_match = 0;

        // set up first iteration
        self.this_row.clear();
        self.last_row.clear();

        for i in a_range {
            // set up our row iteration
            std::mem::swap(&mut self.this_row, &mut self.last_row);
            self.this_row.clear();

            // let's go!
            let a_elem = &self.a[i];

            let Some(b_positions) = self.b_indices.get(a_elem) else {
                continue;
            };

            for &j in b_positions {
                // We already know that a[i] == b[j].

                // Positions in `b_indices` are sorted, so we can skip and then stop once we
                // leave the active search window.
                if j < b_range.start {
                    continue;
                }
                if j >= b_range.end {
                    break;
                }

                // This is the DP recurrence:
                // match_len(i, j) = match_len(i - 1, j - 1) + 1.
                let match_len = if j > 0 { self.last_row[j - 1] + 1 } else { 1 };
                self.this_row[j] = match_len;

                // keep track of the longest match we found so far
                if match_len > longest_match {
                    let longest_a_range = i - (match_len - 1)..i + 1;
                    let longest_b_range = j - (match_len - 1)..j + 1;
                    longest_a_b = Some((longest_a_range, longest_b_range));
                    longest_match = match_len;
                }
            }
        }

        longest_a_b
    }

    /// Extend an anchor match into adjacent equal elements inside the current window.
    ///
    /// This is what lets popular elements re-enter the final equal block: they cannot start a
    /// match when they were filtered out of `b_indices`, but once a nearby non-popular anchor
    /// has been found they are allowed to extend that block as long as equality continues.
    ///
    /// Staying inside `window` is important. Recursive subproblems are defined by the current
    /// unmatched window, so expanding beyond it would make sibling matches overlap.
    fn expand_match_within_window(&self, window: ABRange, match_block: ABRange) -> ABRange {
        let (window_a, window_b) = window;
        let (a_range, b_range) = match_block;

        if a_range.is_empty() || b_range.is_empty() {
            // early exit if we don't have an actual match
            return (a_range, b_range);
        }

        let mut expanded_a = a_range;
        let mut expanded_b = b_range;

        while expanded_a.start > window_a.start
            && expanded_b.start > window_b.start
            && self.a[expanded_a.start - 1] == self.b[expanded_b.start - 1]
        {
            expanded_a.start -= 1;
            expanded_b.start -= 1;
        }

        while expanded_a.end < window_a.end
            && expanded_b.end < window_b.end
            && self.a[expanded_a.end] == self.b[expanded_b.end]
        {
            expanded_a.end += 1;
            expanded_b.end += 1;
        }

        (expanded_a, expanded_b)
    }

    /// Find all equal blocks by recursively splitting around the longest match in each window.
    ///
    /// This is an iterative formulation of the gestalt matching recursion. Each work item is an
    /// unmatched `(a_window, b_window)` pair. When a longest match is found, that window is
    /// split into the unmatched regions on the left and right of the match and those regions are
    /// processed later.
    ///
    /// The returned blocks are sorted and non-overlapping, but they are not guaranteed to be
    /// minimal: two equal blocks may end up adjacent and are left unmerged because [`compare`]
    /// only needs them as separators between unmatched gaps.
    fn find_matching_blocks(&mut self) -> Vec<ABRange> {
        let mut match_blocks = Vec::new();

        // initialize with the whole search window
        let mut work_queue = vec![(0..self.a.len(), 0..self.b.len())];

        while let Some(window) = work_queue.pop() {
            if let Some(match_block) = self.find_longest_match(window.clone()) {
                // Pull adjacent popular elements back into the equal block.
                let match_block = self.expand_match_within_window(window.clone(), match_block);

                match_blocks.push(match_block.clone());

                let left = (
                    window.0.start..match_block.0.start,
                    window.1.start..match_block.1.start,
                );
                let right = (
                    match_block.0.end..window.0.end,
                    match_block.1.end..window.1.end,
                );

                if !left.0.is_empty() && !left.1.is_empty() {
                    work_queue.push(left);
                }

                if !right.0.is_empty() && !right.1.is_empty() {
                    work_queue.push(right);
                }
            }
        }

        match_blocks.sort_by_key(|(a_range, _)| a_range.start);

        match_blocks
    }

    /// Emit operations for the unmatched gap between two equal blocks.
    ///
    /// Gaps are always flattened as deletes from `a` followed by inserts from `b`. This file
    /// does not have a separate `Replace` opcode; replacement behavior is expressed by the
    /// ordering of these two runs.
    fn push_gap_diffops(
        &self,
        diff_ops: &'_ mut Vec<(ChangeTag, &'a T)>,
        (a_range, b_range): ABRange,
    ) {
        for i in a_range {
            diff_ops.push((ChangeTag::Delete, &self.a[i]));
        }
        for j in b_range {
            diff_ops.push((ChangeTag::Insert, &self.b[j]));
        }
    }

    /// Emit operations for a contiguous equal block.
    fn push_equal_diffops(
        &self,
        diff_ops: &'_ mut Vec<(ChangeTag, &'a T)>,
        (a_range, _b_range): ABRange,
    ) {
        for i in a_range {
            diff_ops.push((ChangeTag::Equal, &self.a[i]));
        }
    }
}

/// Compare two slices and return a flat, element-level diff.
///
/// The output is a sequence of `(ChangeTag, &T)` pairs in left-to-right diff order:
///
/// - `ChangeTag::Equal` references an element from `a` that is also present in `b` as part of
///   an equal block.
/// - `ChangeTag::Delete` references an element from `a` that belongs only to the unmatched side
///   of a gap.
/// - `ChangeTag::Insert` references an element from `b` that belongs only to the unmatched side
///   of a gap.
///
/// The matcher is asymmetric because the autojunk heuristic is applied only to `b`. If that
/// matters for your caller, do not assume that `compare(a, b)` and `compare(b, a)` are just
/// inverses of each other.
///
/// Replacement is represented implicitly: if a gap contains elements from both `a` and `b`,
/// this function emits all deletes for that gap first and then all inserts for that gap.
///
/// # Example
///
/// ```rust,ignore
/// use crate::difflib::compare;
/// use crate::utils::ChangeTag;
///
/// let old = ["a", "b", "d"];
/// let new = ["a", "c", "d", "e"];
///
/// let ops = compare(&old, &new);
/// assert_eq!(
///     ops,
///     vec![
///         (ChangeTag::Equal, &"a"),
///         (ChangeTag::Delete, &"b"),
///         (ChangeTag::Insert, &"c"),
///         (ChangeTag::Equal, &"d"),
///         (ChangeTag::Insert, &"e"),
///     ]
/// );
/// ```
pub fn compare<'a, T: Hash + Eq>(a: &'a [T], b: &'a [T]) -> Vec<(ChangeTag, &'a T)> {
    let mut differ = Differ::new(a, b);

    let matching_blocks = differ.find_matching_blocks();

    let mut diff_ops = Vec::with_capacity(a.len() + b.len());
    let (mut last_a, mut last_b) = (0, 0);
    for matching_block in matching_blocks {
        let (a_range, b_range) = matching_block.clone();

        debug_assert!(
            last_a <= a_range.start,
            "matching blocks overlap or are out of order in a: previous end {}, current {:?}",
            last_a,
            a_range
        );
        debug_assert!(
            last_b <= b_range.start,
            "matching blocks overlap or are out of order in b: previous end {}, current {:?}",
            last_b,
            b_range
        );
        debug_assert_eq!(
            a_range.len(),
            b_range.len(),
            "equal blocks must have the same length: {:?} vs {:?}",
            a_range,
            b_range
        );

        differ.push_gap_diffops(
            &mut diff_ops,
            (last_a..a_range.start, last_b..b_range.start),
        );

        differ.push_equal_diffops(&mut diff_ops, matching_block);

        last_a = a_range.end;
        last_b = b_range.end;
    }

    // collect remaining gap to string end
    differ.push_gap_diffops(&mut diff_ops, (last_a..a.len(), last_b..b.len()));

    diff_ops
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// Format diff ops as strings like Python's Differ output for readable assertions.
    fn format_ops<T: std::fmt::Debug>(ops: &[(ChangeTag, &'_ T)]) -> Vec<String> {
        ops.iter()
            .map(|op| {
                let prefix = match op.0 {
                    ChangeTag::Equal => "  ",
                    ChangeTag::Insert => "+ ",
                    ChangeTag::Delete => "- ",
                };
                format!("{prefix}{:?}", op.1)
            })
            .collect()
    }

    /// Extract just the tags for compact assertions.
    fn tags<T>(ops: &[(ChangeTag, &'_ T)]) -> Vec<ChangeTag> {
        ops.iter().map(|op| op.0).collect()
    }

    #[test]
    fn empty_sequences() {
        let empty: &[&str] = &[];
        assert!(compare::<&str>(empty, empty).is_empty());
    }

    #[test]
    fn one_empty_sequence() {
        let a: &[&str] = &["x", "y"];
        let empty: &[&str] = &[];

        let ops = compare(a, empty);
        assert_eq!(tags(&ops), vec![ChangeTag::Delete, ChangeTag::Delete]);

        let ops = compare(empty, a);
        assert_eq!(tags(&ops), vec![ChangeTag::Insert, ChangeTag::Insert]);
    }

    #[test]
    fn identical_sequences() {
        let a = &["a", "b", "c"];
        let ops = compare(a, a);
        assert_eq!(tags(&ops), vec![ChangeTag::Equal; 3]);
    }

    #[test]
    fn completely_different() {
        let a = &["a", "b"];
        let b = &["x", "y", "z"];
        let ops = compare(a, b);
        // Replace: ins_count (3) > del_count (2) → deletes first
        assert_eq!(
            tags(&ops),
            vec![
                ChangeTag::Delete,
                ChangeTag::Delete,
                ChangeTag::Insert,
                ChangeTag::Insert,
                ChangeTag::Insert,
            ]
        );
    }

    #[test]
    fn simple_insert() {
        let a = &["a", "c"];
        let b = &["a", "b", "c"];
        let ops = compare(a, b);
        assert_eq!(
            tags(&ops),
            vec![ChangeTag::Equal, ChangeTag::Insert, ChangeTag::Equal]
        );
        assert_eq!(ops[1].1, &"b");
    }

    #[test]
    fn simple_delete() {
        let a = &["a", "b", "c"];
        let b = &["a", "c"];
        let ops = compare(a, b);
        assert_eq!(
            tags(&ops),
            vec![ChangeTag::Equal, ChangeTag::Delete, ChangeTag::Equal]
        );
        assert_eq!(ops[1].1, &"b");
    }

    #[test]
    fn replace_shorter_deletes_first() {
        // When deletes < inserts, deletes are emitted first.
        let a = &["a", "x", "b"];
        let b = &["a", "w", "y", "z", "b"];
        let ops = compare(a, b);
        assert_eq!(
            format_ops(&ops),
            vec![
                "  \"a\"", "- \"x\"", // 1 delete < 3 inserts → delete first
                "+ \"w\"", "+ \"y\"", "+ \"z\"", "  \"b\"",
            ]
        );
    }

    #[test]
    fn replace_equal_length_deletes_first() {
        // When both sides have equal length, deletes come first.
        // (Python: `bhi - blo < ahi - alo` is strict less-than.)
        let a = &["a", "x", "b"];
        let b = &["a", "y", "b"];
        let ops = compare(a, b);
        assert_eq!(
            format_ops(&ops),
            vec!["  \"a\"", "- \"x\"", "+ \"y\"", "  \"b\""]
        );
    }

    #[test]
    fn python_docstring_example() {
        // From Python difflib docs: comparing "qabxcd" and "abycdf".
        let a: Vec<char> = "qabxcd".chars().collect();
        let b: Vec<char> = "abycdf".chars().collect();
        let ops = compare(&a, &b);

        let formatted: Vec<String> = ops
            .iter()
            .map(|op| {
                let prefix = match op.0 {
                    ChangeTag::Equal => " ",
                    ChangeTag::Insert => "+",
                    ChangeTag::Delete => "-",
                };
                format!("{prefix}{}", op.1)
            })
            .collect();

        assert_eq!(
            formatted,
            vec!["-q", " a", " b", "-x", "+y", " c", " d", "+f"]
        );
    }

    #[test]
    fn no_autojunk_below_200() {
        // With fewer than 200 elements, all elements remain in b2j — even very common ones.
        let b199: Vec<&str> = vec!["pop"; 199];
        let differ = Differ::new(&["pop"], &b199);
        assert!(
            differ.b_indices.contains_key(&"pop"),
            "'pop' should be in b2j when b.len() < 200"
        );

        // Verify the threshold kicks in at exactly 200 elements.
        let b200: Vec<&str> = vec!["pop"; 200];
        let differ = Differ::new(&["pop"], &b200);
        assert!(
            !differ.b_indices.contains_key(&"pop"),
            "'pop' should be excluded from b2j when b.len() >= 200"
        );
    }

    #[test]
    fn autojunk_excludes_popular_from_anchoring() {
        // With >= 200 elements in b, elements appearing > 1% are "popular"
        // and excluded from starting a match (but can extend one).
        //
        // Build b with one element ("pop") appearing ~50% of the time,
        // and one rare element ("rare") appearing once.
        let n = 200;
        let mut b: Vec<&str> = vec!["pop"; n];
        b[100] = "rare";

        // a has just ["pop", "rare", "pop"].
        let a = &["pop", "rare", "pop"];

        let differ = Differ::new(a, &b);

        // "pop" should be excluded from b2j (it's in >1% of positions).
        assert!(
            !differ.b_indices.contains_key(&"pop"),
            "popular element 'pop' should be excluded from b2j"
        );

        // "rare" should still be in b2j.
        assert!(
            differ.b_indices.contains_key(&"rare"),
            "rare element should remain in b2j"
        );
    }

    #[test]
    fn autojunk_extends_into_popular_tokens() {
        // After finding a match on a non-popular anchor, the match should
        // extend into adjacent popular tokens via the extension step.
        let n = 200;
        let mut b: Vec<&str> = vec!["pop"; n]; // all "pop" (200 times → popular)
        b[50] = "rare_left";
        b[51] = "anchor";
        b[52] = "rare_right";

        let a = &["pop", "rare_left", "anchor", "rare_right", "pop"];

        let mut differ = Differ::new(a, &b);
        // "pop" is popular (appears 197 times > 200/100+1 = 3 threshold)
        assert!(!differ.b_indices.contains_key(&"pop"));

        // find_longest_match should find "rare_left anchor rare_right" (size 3)
        // and then expand_match_within_window into the adjacent "pop" on both sides (size → 5).
        let match_block = differ
            .find_longest_match((0..a.len(), 0..b.len()))
            .expect("match should be found");
        let (a_range, b_range) =
            differ.expand_match_within_window((0..a.len(), 0..b.len()), match_block);
        assert_eq!(
            a_range.len(),
            5,
            "match should extend into adjacent popular 'pop' tokens"
        );
        assert_eq!(a_range.start, 0);
        assert_eq!(b_range.start, 49); // b[49]="pop", b[50..53]=rare_left/anchor/rare_right, b[53]="pop"
    }

    #[test]
    fn works_with_integers() {
        // Verify the generic works with non-string types.
        let a = &[1, 2, 3, 4, 5];
        let b = &[1, 3, 4, 6];
        let ops = compare(a, b);

        assert_eq!(
            ops,
            vec![
                (ChangeTag::Equal, &1),
                (ChangeTag::Delete, &2),
                (ChangeTag::Equal, &3),
                (ChangeTag::Equal, &4),
                (ChangeTag::Delete, &5),
                (ChangeTag::Insert, &6),
            ]
        );
    }

    #[test]
    fn repeated_elements() {
        // Test with many repeated elements to verify correct matching behavior.
        let a = &["a", "b", "a", "b", "c"];
        let b = &["a", "b", "c", "a", "b"];
        let ops = compare(a, b);

        let formatted: Vec<String> = ops
            .iter()
            .map(|op| {
                let c = match op.0 {
                    ChangeTag::Equal => ' ',
                    ChangeTag::Insert => '+',
                    ChangeTag::Delete => '-',
                };
                format!("{c}{}", op.1)
            })
            .collect();

        // The longest contiguous match is a[2..5]="a b c" matching b[0..3]="a b c" (size 3).
        // Left of match: a[0..2] vs b[0..0] → a[0..2] deleted.
        // Right of match: a[5..5] vs b[3..5] → b[3..5] inserted.
        assert_eq!(formatted, vec!["-a", "-b", " a", " b", " c", "+a", "+b"]);
    }

    #[test]
    fn matching_blocks_do_not_overlap() {
        // Regression test: recursive subproblems must not yield overlapping matching
        // blocks. A previous implementation expanded matches beyond the active window,
        // which caused an overlap in `a` for this input.
        let a = &["a", "b", "a"];
        let b = &["a", "b", "b", "a"];
        let mut differ = Differ::new(a, b);

        let matches = differ.find_matching_blocks();

        assert_eq!(matches.len(), 2);

        let first = &matches[0];
        let second = &matches[1];

        assert!(
            first.0.end <= second.0.start,
            "matching blocks overlap in a: {matches:?}"
        );
        assert!(
            first.1.end <= second.1.start,
            "matching blocks overlap in b: {matches:?}"
        );
    }
}
