//! Pure Rust port of Python's `difflib.Differ.compare()`.
//!
//! This module implements the Ratcliff/Obershelp "gestalt pattern matching" algorithm
//! as used in Python's standard library `difflib`. The key entry point is [`compare`],
//! which produces a flat sequence of element-level diff operations (equal, insert, delete).
//!
//! # Why this algorithm?
//!
//! WikiWho's authorship attribution pipeline needs a diff algorithm that:
//!
//! 1. **Prefers contiguous matches.** The algorithm finds the longest contiguous matching
//!    block, then recursively processes the regions to its left and right. This strongly
//!    favors matches that preserve local structure — tokens are only matched if they appear
//!    in the same local context. LCS-based algorithms (Myers, Patience) may align tokens
//!    across unrelated parts of the text, leading to wrong attribution.
//!
//! 2. **Filters popular (high-frequency) tokens.** With the "autojunk" heuristic, elements
//!    that appear in more than 1% of positions in `b` are excluded from the initial match
//!    search. This prevents common tokens like `[[`, `the`, or `is` from acting as false
//!    anchors. After a contiguous match is found, it is extended into adjacent popular
//!    tokens, so popular tokens that moved together with their context retain provenance.
//!
//! # Algorithm overview
//!
//! 1. **Build index** ([`SequenceMatcher::new`]): For each element in `b`, record the
//!    positions where it appears. Remove "popular" elements (autojunk heuristic).
//!
//! 2. **Find matches** ([`SequenceMatcher::find_longest_match`]): Sweep through `a`,
//!    tracking contiguous match lengths against `b` positions. Extend the best match
//!    into adjacent popular elements.
//!
//! 3. **Recurse** ([`SequenceMatcher::matching_blocks`]): Apply step 2 recursively
//!    (iteratively via a stack) to the regions left and right of each match.
//!
//! 4. **Produce opcodes** ([`SequenceMatcher::opcodes`]): Convert matching blocks into
//!    range-based edit operations (equal, insert, delete, replace).
//!
//! 5. **Flatten** ([`compare`]): Expand range-based opcodes into per-element `DiffOp`s,
//!    applying the Differ's replace-ordering rule (shorter block first).

// note: for complexity we use only n instead of n and m since we assume the input sequences have similar length
// (e.g. instead of n+m it is 2n)

use std::{hash::Hash, ops::Range};

use rustc_hash::FxHashMap;

use crate::utils::ChangeTag;

// ---------------------------------------------------------------------------
// ClearableArray
// ---------------------------------------------------------------------------

/// A dense `usize` array that supports O(1) logical clearing via a generation counter.
///
/// Conceptually behaves like a `Vec<usize>` initialized to all zeros. Calling [`clear`]
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
    /// The current generation. Incremented by [`clear`].
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

type ABRange = (Range<usize>, Range<usize>);

// LCSt is used for longest common subSTRING here (don't confuse it with LCS = longest common subsequence)
struct Differ<'a, T: Hash + Eq> {
    a: &'a [T],
    b: &'a [T],

    b_indices: FxHashMap<&'a T, Vec<usize>>,
    last_row: ClearableArray,
    this_row: ClearableArray,
}

impl<'a, T: Hash + Eq> Differ<'a, T> {
    fn new(a: &'a [T], b: &'a [T]) -> Self {
        Self {
            a,
            b,
            b_indices: Self::build_elem_indices(b),
            last_row: ClearableArray::new(b.len()),
            this_row: ClearableArray::new(b.len()),
        }
    }

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

    /// This function returns a range for `self.a` and `self.b` where the
    /// longest common contiguous match - that does not include any junk! -
    /// of the slices `&self.a[a_range]` and `&self.b[b_range]` is found.
    ///
    /// If no such match is found, None is returned.
    /// If Some is returned the returned ranges are always of non-0 length.
    ///
    /// ## Algorithm
    /// In our algorithm we are looking at index pairs (i, j), where i is an index of a and j is an index of b.
    ///
    /// ### Step 1 - Visualisation
    /// There are multiple ways you might go about trying to find matching runs in two strings.
    /// An intuitive approach might be to write the sequences on two strips of paper and slide them
    /// along each other, checking where the characters on the top and bottom strip match.
    ///
    /// ```text
    /// Strip a:             B C D      =>    B C D     =>      B C D
    /// Strip b:             X B C D Y  =>  X B C D Y   =>  X B C D Y
    /// Mentally counting:   0 0 0 0 0  =>  0 1 2 3 0   =>  0 0 0 0 0
    /// (We are skipping a few possible combinations here for brevity)
    /// ```
    ///
    /// Let's take a closer look at that mental counting we might have been doing there,
    /// it is about to get important!
    /// (In case you usually read right-to-left observe how we began counting from the
    /// left-most matching character here!)
    ///
    /// We found a nice match when we moved the strips so that a[i=0] matches up with b[j=1],
    /// a[i=1] with b[j=2], a[i=2] with b[j=3], ... you get the idea.
    ///
    /// Let's note these observations down in a table :)
    /// ```text
    /// +-------+-------+-------+-------+-------+-------+
    /// | i / j | j = 0 | j = 1 | j = 2 | j = 3 | j = 4 |
    /// +-------+-------+-------+-------+-------+-------+
    /// | i = 0 |     0 |     1 |     0 |       |       |
    /// | i = 1 |       |     0 |     2 |     0 |       |
    /// | i = 2 |       |       |     0 |     3 |     0 |
    /// +-------+-------+-------+-------+-------+-------+
    /// ```
    /// (The empty cells are theoretically also 0 - I left them out to make it easier to follow)
    ///
    /// Take a moment to understand the table.
    /// Our algorithm is conceptually looking for the **highest cell value** in **this table**!
    ///
    /// You can understand each cell value as an answer to the question:
    /// "How many matching elements will I encounter when I walk **backwards** through a and b,
    /// starting from element a[i] and b[j] respectively?"
    /// (_nb: totally what I'm asking myself each morning_)
    /// An alternative wording would be "How long is the match 'ending' at this position?"
    /// (shorter but maybe not as intuitive)
    ///
    /// If you are still lost, maybe it helps to look at the diagonals of this table. Each of those
    /// describes a single "shift state", i.e. what the world (or in this case your mental couting)
    /// looks like when you align strip a and b in one distinct combination.
    ///
    /// ### Step 2 - Basic algorithm
    /// Now that we have this table form we may notice a pattern here:
    /// If characters at a[i] and b[j] match then we increment by one, otherwise we note down `0`.
    /// Incrementing _a number_ means we need _a number_ to increment. In our case that number is
    /// taken from the cell at (i - 1, j - 1) (i.e. following the diagonal towards the upper left)
    /// if it exists - otherwise `0`.
    ///
    /// So we can formulate a simple recursive algorithm:
    /// ```text
    /// FUNCTION match_len (i, j)
    /// IF exists(a[i]) AND exists(b[j]) AND a[i] == b[j] THEN
    ///     RETURN match_len(i - 1, j - 1) + 1
    /// ELSE
    ///     RETURN 0
    /// END FUNCTION
    /// ```
    ///
    /// ### Step 3 - Optimization
    /// If we evaluate this non-recursively the simplest approach would be iterating through j
    /// for every i, i.e. filling the table row by row.
    /// This would make sure that when evaluating a cell we are sure to have already evaluated
    /// the dependency at (i - 1, j - 1). (Row -1 and column -1 are conceptually initialized with zeroes.)
    ///
    /// Since our diff algorithm ignores elements that occur often we can expect the a[i] == b[j]
    /// comparison to hold true in only a small amount of cases. So instead of iterating through the whole space
    /// of j we build an index up front. This index can tell us:
    /// "Where in b can we find an equal element to a[i]?"
    /// Then we only iterate through those indices of b returned by our query and evaluate
    /// `match_len(i, j) = match_len(i - 1, j - 1) + 1` for each.
    /// All other cells in this row are (implicitely) set to `0`.
    ///
    /// We end up still iterating through all **rows** but for each row we only iterate through
    /// a small fraction of the **columns**.
    ///
    /// This has the added benefit of making it very easy to implement the property of our algorithm
    /// to not match on junk: We simply don't include the junk in our index so if the element at
    /// `a[i]` is junk we just never get any matches in b to iterate over.
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

            for &j in self.b_indices.get(a_elem).unwrap_or(&Vec::new()) {
                // here we already know that a[i] == b[j]

                // b_indices are sorted - ignore matches outside our `b_range`
                if j < b_range.start {
                    continue;
                }
                if j >= b_range.end {
                    break;
                }

                // this is our `match_len(i, j) = match_len(i - 1, j - 1) + 1`
                let match_len = if j > 0 {
                    self.last_row[j - 1] + 1
                } else {
                    1
                };
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

    // may yield equal blocks that are side-by-side, i.e. returned list is not minimal
    // iterative implementation of the gestalt pattern matching algorithm
    fn find_matching_blocks(&mut self) -> Vec<ABRange> {
        let mut match_blocks = Vec::new();

        // initialize with the whole search window
        let mut work_queue = vec![(0..self.a.len(), 0..self.b.len())];

        while let Some(window) = work_queue.pop() {
            match self.find_longest_match(window.clone()) {
                Some(match_block) => {
                    // take as many adjacent autojunk tokens as possible
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
                None => {}
            }
        }

        match_blocks.sort_by_key(|(a_range, _)| a_range.start);

        match_blocks
    }

    /// Emit diffops for a gap (the region between two matching blocks), if non-empty.
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

    /// Emit diffops for an equal block.
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

pub fn compare<'a, T: Hash + Eq>(a: &'a [T], b: &'a [T]) -> Vec<(ChangeTag, &'a T)> {
    let mut differ = Differ::new(a, b);

    let matching_blocks = differ.find_matching_blocks();

    let mut diff_ops = Vec::new();
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
