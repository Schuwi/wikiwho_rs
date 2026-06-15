# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.3.2] - 2026-06-15

### Added

- Releases are now built and published by CI with a verifiable [SLSA build-provenance attestation](https://github.com/actions/attest-build-provenance); published crates can be checked with `gh attestation verify` (see "Verifying a release" in the README).

## [0.3.1] - 2026-04-05

### Fixed

- `python-diff` exact-comparison mode now uses Python's Unicode lowercasing for non-ASCII text, avoiding false divergences caused by Python and Rust using different Unicode table versions.

### Changed

- Refreshed dependency versions across the crate and test tooling, including upgrades to `pyo3`, `quick-xml`, `imara-diff`, `rand`, `bincode`, and related supporting crates.

## [0.3.0] - 2026-04-05

### Added

- `wikiwho-cli` binary (behind new `cli` feature flag): a full-featured command-line tool for running WikiWho analysis on MediaWiki XML dumps. Supports compressed input/output (bzip2, gzip, zstd) with auto-detection from file extension, stdin/stdout piping, namespace filtering, three output formats (JSONL, JSON, raw), progress reporting, page count limit (`--limit`), and multi-threaded processing (`-j` flag, defaults to number of CPUs).
- `wikiwho-viewer.html`: a standalone single-page HTML tool for viewing CLI output.
- `PageAnalysisOptions` struct for selecting algorithm options at runtime (e.g., `optimize_non_ascii` behind `optimized-lowercase`, `use_python_diff` behind `python-diff`).

### Changed

- **Breaking:** `RevisionImmutables` no longer stores the full `dump_parser::Revision`. It now exposes only the revision `id: i32`. The `Deref<Target = Revision>` impl has been removed, so code accessing `Revision` fields through a `RevisionPointer` will no longer compile.
- **Breaking:** `optimized-str` feature semantics changed. Unicode lowercasing optimization has been split out into a new `optimized-lowercase` feature, since unlike `optimized-str` it is only beneficial situationally (inputs with a high proportion of non-ASCII). Both features are now purely additive, consistent with Cargo conventions: they add optimized implementations rather than toggling behavior. `optimized-str` is now enabled by default and brings in the `aho-corasick` and `memchr` dependencies.
- **Breaking:** `python-diff` feature no longer automatically uses Python's diff algorithm when enabled. Opt in at runtime via `PageAnalysisOptions::use_python_diff`.
- **Breaking (serde):** Serialized `PageAnalysis` format has changed due to a new zero-copy internal representation. Previously serialized data is not compatible.
- `analyse_page` and `analyse_page_with_options` now accept `impl IntoIterator<Item: Borrow<Revision>>` instead of `&[Revision]`. Existing call sites passing a slice still compile without changes.
- Significantly reduced memory usage through zero-copy substring handling using the `yoke` crate. Text content is now shared across the analysis data structure rather than duplicated.
- Raw revision text is freed immediately after a revision is processed, reducing peak memory consumption during analysis.
- Serialization avoids unnecessary `String` cloning.

## [0.2.0] - 2026-03-22

### Added

- Optional `serde` feature flag for serialization/deserialization of `PageAnalysis` and related types.
- `DumpParser::parse_single_page` method for parsing a single page from an XML reader.
- `Debug` impl for algorithm pointer types.
- `ParsingError::MissingField` and `ParsingError::MismatchedTags` variants (behind `strict` feature).
- Doc comments across the public API.

### Changed

- **Breaking:** `Analysis` renamed to `PageAnalysis`.
- **Breaking:** `PageAnalysis::words` changed from `Vec<WordAnalysis>` to `Vec<WordPointer>`. Use the `Index` impl on `PageAnalysis` with a `WordPointer` to access word data.
- **Breaking:** `PageAnalysis::revisions`, `paragraphs`, and `sentences` fields changed from `pub` to `pub(crate)`. Use the `Index` impl on `PageAnalysis` with the corresponding pointer type instead.
- **Breaking:** Tuple fields on `RevisionPointer`, `ParagraphPointer`, `SentencePointer`, and `WordPointer` changed from `pub` to `pub(crate)`. Use `Deref` (for immutable data) or `Pointer` trait methods instead.
- **Breaking:** `ParsingError` is now `#[non_exhaustive]`.
- `Sha1Hash` inner field is now publicly accessible.
- Internal data types extracted into separate modules.

## [0.1.0] - 2024-10-20

Initial release.

[unreleased]: https://github.com/Schuwi/wikiwho_rs/compare/v0.3.1...HEAD
[0.3.1]: https://github.com/Schuwi/wikiwho_rs/compare/v0.3.0...v0.3.1
[0.3.0]: https://github.com/Schuwi/wikiwho_rs/compare/v0.2.0...v0.3.0
[0.2.0]: https://github.com/Schuwi/wikiwho_rs/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/Schuwi/wikiwho_rs/releases/tag/v0.1.0
