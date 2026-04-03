# Intuition

There are multiple ways to think about how to find matching runs in two sequences. An intuitive
one is to imagine writing the sequences on two strips of paper and sliding them past each other,
checking where equal elements line up.

For example, if `a = [B, C, D]` and `b = [X, B, C, D, Y]`, a few alignments look like this:

```text
Strip a:        B C D       =>    B C D     =>      B C D
Strip b:        X B C D Y   =>  X B C D Y   =>  X B C D Y
Mental count:   0 0 0 0 0   =>  0 1 2 3 0   =>  0 0 0 0 0
```

(Only a few alignments are shown here for brevity.)

The interesting part is that "mental count" row. In the middle alignment, `a[0]` lines up with
`b[1]`, `a[1]` with `b[2]`, and `a[2]` with `b[3]`, so the running count becomes `1, 2, 3`.

That same idea can be written down more systematically in a table over all index pairs `(i, j)`,
where `i` is an index into `a` and `j` is an index into `b`:

```text
+-------+-------+-------+-------+-------+-------+
| i / j | j = 0 | j = 1 | j = 2 | j = 3 | j = 4 |
+-------+-------+-------+-------+-------+-------+
| i = 0 |     0 |     1 |     0 |       |       |
| i = 1 |       |     0 |     2 |     0 |       |
| i = 2 |       |       |     0 |     3 |     0 |
+-------+-------+-------+-------+-------+-------+
```

Empty cells are also conceptually `0`; they are omitted here to keep the picture readable.

Each diagonal in this table corresponds to one particular alignment of the two strips. The
algorithm is conceptually looking for the largest value anywhere in this table.

Each cell answers one question:

> How many matching elements will I encounter when I walk backwards through `a` and `b`,
> starting from element `a[i]` and `b[j]` respectively?

In short, each cell contains _the length of the contiguous match ending at `(i, j)`_.

The key observation is that a run ending at `(i, j)` can only extend a run ending at
`(i - 1, j - 1)`. So the recurrence is:

```text
match_len(i, j) =
    if a[i] == b[j] { match_len(i - 1, j - 1) + 1 } else { 0 }
```

The algorithm is therefore looking for the maximum value in that implicit table. The cell with
the largest value identifies the longest contiguous anchor match in the current search window.

# Why the implementation is sparse

Filling the whole table row by row would work, but it would also do a lot of pointless work:
most `(i, j)` pairs do not match.

Instead, `b_indices` answers this narrower question:

> For the current `a[i]`, at which positions `j` in `b` could `a[i] == b[j]` possibly hold?

That means the loop only visits cells that could evaluate non-zero. All other cells in the implicit
table remain zero without being stored explicitly.

This is also how the autojunk heuristic plugs into the algorithm. If an element is too common
in `b`, it is removed from `b_indices`, so it cannot start or continue a match. Later,
[`expand_match_within_window`](Self::expand_match_within_window) is allowed to grow a found
match into adjacent popular elements.

# How that maps to the code

- `last_row[j - 1]` stores the value for the diagonal predecessor `(i - 1, j - 1)`.
- `this_row[j]` stores the value for the current cell `(i, j)`.
- `longest_match` tracks the largest value seen so far.
- When a new maximum is found, the code reconstructs the corresponding `a` and `b` ranges from
  the current end positions `i`, `j`, and the run length.
