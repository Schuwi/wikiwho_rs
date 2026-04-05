# Wikimedia-Derived Article Cache Fixtures

This directory is for serialized page fixtures generated from the shared development data flow used by `tests/algorithm_statistic_tests.rs` and related benchmarks.

## Format

- One file per article, typically named `<Article>.json.zst`
- Content is derived from Wikimedia dump XML and transformed into the crate's serialized `Page` representation
- Compression is `zstd`

## Licensing

Files committed here are Wikimedia-derived text data, not software covered by the repository's MPL/MIT code license.

For Wikimedia text dumps, the relevant reuse guidance is generally:

- Creative Commons Attribution-ShareAlike 4.0
- GNU Free Documentation License

That means committed fixtures should retain clear attribution metadata and should be treated as share-alike text derivatives, not relicensed as project code.

## Attribution Practice

When committing cache files here:

- keep this file in the directory
- update `ATTRIBUTION.md`
- include one entry per committed cache file
- provide article and history URLs so attribution can be traced back to Wikimedia page history
- mention that the file is a transformed dump extract serialized to `.json.zst`

Do not put unrelated local-only caches here.
