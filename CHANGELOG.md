# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

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

[0.2.0]: https://github.com/Schuwi/wikiwho_rs/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/Schuwi/wikiwho_rs/releases/tag/v0.1.0
