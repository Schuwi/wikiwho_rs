// SPDX-License-Identifier: MPL-2.0
//! Visible "this suite was skipped" notices for the feature-gated integration tests.
//!
//! `algorithm_exact_tests`, `algorithm_statistic_tests`, and `utils_comparisons`
//! are gated behind cargo features at the *file* level (e.g.
//! `#![cfg(all(feature = "python-diff", feature = "serde"))]`). Without those features
//! each one compiles to an empty test binary that prints `running 0 tests` — which is
//! indistinguishable from a passing run, and has fooled contributors into thinking a
//! plain `cargo test` exercised the Rust-vs-Python parity suites when it actually ran
//! nothing.
//!
//! Each test below is compiled *only* when its suite is gated out, so a plain
//! `cargo test` surfaces a named, passing test that says exactly which features to
//! enable. The test name shows in libtest's default output; run with `--nocapture` for
//! the full `SKIP:` line. See `CONTRIBUTING.md` → "Running the tests".

/// `tests/algorithm_exact_tests.rs` — token-level Rust-vs-Python parity.
/// Needs `python-diff` (same diff algorithm as the reference) and `serde`.
#[cfg(not(all(feature = "python-diff", feature = "serde")))]
#[test]
fn skipped_algorithm_exact_tests_enable_python_diff_and_serde() {
    eprintln!(
        "SKIP: algorithm_exact_tests compiled empty — run with \
         `--features python-diff,serde` to check Rust-vs-Python parity (see CONTRIBUTING.md)."
    );
}

/// `tests/algorithm_statistic_tests.rs` — gold-standard accuracy (uses the serde JSON cache).
#[cfg(not(feature = "serde"))]
#[test]
fn skipped_algorithm_statistic_tests_enable_serde() {
    eprintln!(
        "SKIP: algorithm_statistic_tests compiled empty — run with `--features serde` \
         to check gold-standard accuracy (see CONTRIBUTING.md)."
    );
}

/// `tests/utils_comparisons.rs` — Rust-vs-Python tokenizer/splitter parity (proptest).
#[cfg(not(feature = "python-diff"))]
#[test]
fn skipped_utils_comparisons_enable_python_diff() {
    eprintln!(
        "SKIP: utils_comparisons compiled empty — run with `--features python-diff` \
         to check Rust-vs-Python tokenizer parity (see CONTRIBUTING.md)."
    );
}
