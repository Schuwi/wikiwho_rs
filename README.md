<div class="rustdoc-hidden">

# wikiwho

</div>

A high-performance Rust implementation of the WikiWho algorithm for token-level authorship tracking in Wikimedia pages.

<div class="rustdoc-hidden">

[![CI](https://github.com/Schuwi/wikiwho_rs/actions/workflows/ci.yml/badge.svg)](https://github.com/Schuwi/wikiwho_rs/actions/workflows/ci.yml)
[![codecov](https://codecov.io/gh/Schuwi/wikiwho_rs/branch/main/graph/badge.svg)](https://codecov.io/gh/Schuwi/wikiwho_rs)
[![crates.io](https://img.shields.io/crates/v/wikiwho.svg)](https://crates.io/crates/wikiwho)
[![docs.rs](https://docs.rs/wikiwho/badge.svg)](https://docs.rs/wikiwho)

</div>

## Overview

`wikiwho` is a Rust library that implements the [WikiWho algorithm](https://github.com/wikiwho/WikiWho), enabling users to track authorship on a token level (token ≈ word) across all revisions of a Wikimedia page (e.g., Wikipedia, Wiktionary). It reimplements the original algorithm by Fabian Flöck and Maribel Acosta with significant performance improvements: it processes an entire German Wiktionary dump (~1.3 million pages) in just under 4 minutes on 8 cores, where the original Python implementation managed roughly 300 pages per minute.

**Key Features:**

- **High Performance**: Processes large dumps in minutes instead of days.
- **Parallel Processing**: Designed for easy parallelization, leveraging Rust's concurrency capabilities.
- **Modular Design**: Separate parser and algorithm modules that can be used independently.
- **Faithful Implementation**: Aims to provide results comparable to the original algorithm, with an option to use the original Python diff algorithm for exact comparisons.

## Validation

CI verifies exact token-level parity against the reference Python WikiWho on every PR, and ≥85% precision against the paper's gold standard (the paper reports ~95% using Python's `difflib`; enable the `python-diff` feature for byte-identical results). Property-test fuzzing additionally checks Rust-vs-Python parity on randomized input. See [`CONTRIBUTING.md`](CONTRIBUTING.md) for how to run these tests locally.

## Quick Start (no Rust required)

`wikiwho` ships a command-line tool, `wikiwho-cli`, that runs the full algorithm over a MediaWiki XML dump and streams the per-page authorship results out as JSON. You only need a Rust toolchain to install it once (see [rustup.rs](https://rustup.rs/)); after that it is an ordinary binary — no Rust knowledge needed to use it.

```sh
# One-time install (the CLI lives behind the `cli` feature)
cargo install wikiwho --features cli

# Analyse a dump. Input compression (.bz2/.zst/.gz) is auto-detected from the
# extension; `--namespace 0` keeps only article pages; results go to out.jsonl.
wikiwho-cli dewiktionary-latest-pages-meta-history.xml.bz2 --namespace 0 -o out.jsonl
```

The input is a standard `*-pages-meta-history*` export from [Wikimedia dumps](https://dumps.wikimedia.org/); omit the path (or pass `-`) to read from stdin. Besides `--namespace` and `-o`, the common flags are `-f/--format` (`jsonl` (default), `json`, or `raw`), `-j/--jobs`, `-N/--limit` (first N pages) and `-q/--quiet` — run `wikiwho-cli --help` for the full list.

Once you have `out.jsonl`, drop it onto [`tools/wikiwho-viewer.html`](tools/wikiwho-viewer.html), a self-contained drag-and-drop browser viewer that colours each token by its author and age (no server or build step).

### Output format

With the default `jsonl` format, each line is one self-contained JSON object describing a page. One page looks like this (truncated):

```json
{
  "article_title": "Anontalkpagetext",
  "namespace": 8,
  "revisions": [
    { "id": 401685, "timestamp": "2006-09-19T20:46:45+00:00", "editor": "1390" },
    { "id": 552578, "timestamp": "2007-05-23T15:43:05+00:00", "editor": "1390" }
  ],
  "spam_ids": [],
  "all_tokens": [
    { "token_id": 0, "str": "/", "o_rev_id": 401685, "editor": "1390", "in": [], "out": [] },
    { "token_id": 1, "str": "span", "o_rev_id": 401685, "editor": "1390", "in": [], "out": [] }
  ]
}
```

`revisions` lists the page's revisions in chronological order; `all_tokens` lists every token (token ≈ word) surviving in the current revision, in reading order. The less obvious fields:

- **`editor`** — user id as a string, or `"0|<username>"` for anonymous/IP edits.
- **`spam_ids`** — revision ids flagged as spam/vandalism and excluded from attribution.
- **`o_rev_id`** / **`editor`** (on a token) — the revision and author that *first introduced* it; this is the authorship attribution.
- **`in`** / **`out`** — revision ids where the token was re-inserted / removed, tracking tokens deleted and later restored.

Because `jsonl` is one JSON object per line, you can load it in any language without a streaming parser. In Python:

```python
import json

with open("out.jsonl") as f:
    for line in f:
        page = json.loads(line)
        print(page["article_title"], len(page["all_tokens"]), "tokens")
```

## Installation (as a library)

`wikiwho` is also available on [crates.io](https://crates.io/crates/wikiwho) as a library. Add it to your `Cargo.toml`:

```toml
[dependencies]
wikiwho = "0.3"
```

Requires Rust ≥ 1.94.1 (MSRV). The only feature enabled by default is `optimized-str`; see
[Features and Configuration](#features-and-configuration) for the full list.

## Usage

### Basic Example

Here's a minimal example of how to load a Wikimedia XML dump and analyze a page:

```rust,no_run
use wikiwho::dump_parser::{Contributor, DumpParser};
use wikiwho::algorithm::PageAnalysis;
use wikiwho::utils::iterate_revision_tokens;
use std::collections::HashMap;
use std::fs::File;
use std::io::BufReader;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let xml_dump = File::open("path/to/pages-meta-history.xml")?;
    let reader = BufReader::new(xml_dump);
    let mut parser = DumpParser::new(reader)?;

    // Parse and analyze a single page.
    if let Some(page) = parser.parse_page()? {
        let analysis = PageAnalysis::analyse_page(&page.revisions)?;

        // The analysis records the *id* of the revision that introduced each token
        // (`analysis.revisions_by_id` maps those ids to per-revision analysis data).
        // Editor names, however, live on the parsed revisions, so build a quick
        // id -> contributor lookup from them:
        let contributors: HashMap<i32, &Contributor> = page
            .revisions
            .iter()
            .map(|rev| (rev.id, &rev.contributor))
            .collect();

        // Walk the tokens of the current (latest) revision and print who first
        // introduced each one.
        for token in iterate_revision_tokens(&analysis, &analysis.current_revision) {
            let origin_id = analysis[token].origin_revision.id;
            let contributor = contributors[&origin_id];

            // For anonymous edits `contributor.id` is `None` and
            // `contributor.username` holds the editor's IP address.
            println!("'{}' by '{}'", token.value.as_str(), contributor.username);
        }
    }

    Ok(())
}
```

### Processing an Entire Dump

To process a full dump, you can iterate over all pages:

```rust,no_run
use wikiwho::dump_parser::DumpParser;
use wikiwho::algorithm::PageAnalysis;
use std::fs::File;
use std::io::BufReader;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let xml_dump = File::open("path/to/pages-meta-history.xml")?;
    let reader = BufReader::new(xml_dump);
    let mut parser = DumpParser::new(reader)?;

    while let Some(page) = parser.parse_page()? {
        // Analyze each page (can be parallelized)
        let analysis = PageAnalysis::analyse_page(&page.revisions)?;

        // Your processing logic here
    }

    Ok(())
}
```

### Parallel Processing

XML parsing is inherently linear, but analysis is independent per page and parallelizes
cleanly. The usual pattern is:

- Run the parser on a single thread.
- Hand each parsed `Page` off to a worker pool.
- Call `PageAnalysis::analyse_page` on each page in parallel and collect the results.

The simplest approach is to feed parsed pages into a [`rayon`](https://docs.rs/rayon)
parallel iterator, or into an `std::sync::mpsc` channel drained by a pool of worker
threads. For a complete, production-grade reference — bounded queueing, progress
reporting, and ordered output — see the bundled CLI in
[`src/bin/wikiwho-cli.rs`](src/bin/wikiwho-cli.rs).

## Modules and API

### `dump_parser`

- **Purpose**: Parses Wikimedia XML dumps.
- **Usage**: Create a `DumpParser` instance with a reader, then call `parse_page()` to retrieve pages one by one.

### `algorithm`

- **Purpose**: Implements the WikiWho algorithm.
- **Usage**: Call `PageAnalysis::analyse_page(&page.revisions)` to analyze the revisions of a page.

### `utils`

- **Purpose**: Provides utility functions.
- **Key Function**: `iterate_revision_tokens()` for easy iteration over tokens in a revision.

## Migrating from Python WikiWho

Coming from the original [Python WikiWho](https://github.com/wikiwho/WikiWho)? The main structural change is token lookup: instead of indexing `Wikiwho.tokens` by position (or by `token_id` in the JSON API), you index the `PageAnalysis` itself with a `WordPointer` (`analysis[word_pointer]`) to get a [`WordAnalysis`](https://docs.rs/wikiwho/latest/wikiwho/algorithm/struct.WordAnalysis.html). Iterate a revision's tokens in order with [`utils::iterate_revision_tokens`](https://docs.rs/wikiwho/latest/wikiwho/utils/fn.iterate_revision_tokens.html) (see the [Basic Example](#basic-example)).

The table maps both ways you might know WikiWho today: the in-process Python object attributes returned by `analyse_article_from_xml_dump`, and the field names from the [WikiWho web API](https://wikiwho-api.wmcloud.org/) (e.g. the `all_content` endpoint).

| Python object | WikiWho JSON API | This crate |
| --- | --- | --- |
| `Wikiwho(title).analyse_article_from_xml_dump(page)` | `all_content` / `rev_content` endpoint | [`PageAnalysis::analyse_page(&page.revisions)`](https://docs.rs/wikiwho/latest/wikiwho/algorithm/struct.PageAnalysis.html#method.analyse_page) |
| `Wikiwho.tokens[i]` | per-token object (`token_id`) | `analysis[word_pointer]` |
| token `.value` | `str` | the token text (`word_pointer.value`) |
| token `.origin_rev_id` | `o_rev_id` | `WordAnalysis.origin_revision.id` |
| token `.inbound` / `.outbound` | `in` / `out` | `WordAnalysis.inbound` / `WordAnalysis.outbound` |
| origin revision's editor | `editor` | contributor of `origin_revision` (see [Basic Example](#basic-example)) |
| `Wikiwho.spam_ids` | *(not exposed)* | `PageAnalysis.spam_ids` |

Behavior matches the Python implementation: paragraph/sentence/token splitting and spam detection use the same logic and constants, and the `python-diff` feature makes results byte-identical to the reference Python WikiWho (the default backend holds ≥85% precision against the paper's gold standard — see [Validation](#validation)).

## Dependencies

The crate keeps a modest set of mandatory dependencies and pulls in the rest only when you
enable the corresponding [feature](#features-and-configuration).

**Always compiled:** `blake3`, `chrono`, `compact_str`, `imara-diff`, `quick-xml`, `rand`,
`regex`, `rustc-hash`, `string-interner`, `thiserror`, `tracing`, and `yoke`. `compact_str`
in particular appears in the public API for efficient handling of mostly short strings such
as page titles and contributor names.

**Optional (feature-gated):** `aho-corasick` + `memchr` (`optimized-str`),
`unicode-case-mapping` (`optimized-lowercase`), `pyo3` (`python-diff`), `serde` +
`serde_json` (`serde`), and `getopts` + `bzip2` + `flate2` + `zstd` (`cli`).

## Performance Considerations

- **Parallel Analysis**: Users are encouraged to implement parallel processing for the analysis phase to maximize performance.
- **Parsing Bottleneck**: XML parsing is linear and may become a bottleneck. Running the parser in a single thread and distributing analysis can optimize performance.
- **Memory Usage**: The parser processes one page at a time, so memory usage is constant relative to the dump size. Ensure you drop processed `Page` and `PageAnalysis` structs to free memory.
- **Diff Algorithm Choice**: By default, a faster diff algorithm is used. For exact results matching the original implementation, enable the `python-diff` feature and use `PageAnalysis::analyse_page_with_options` to select the Python diff algorithm.

## Features and Configuration

`wikiwho` exposes six Cargo features. Only `optimized-str` is enabled by default
(`default = ["optimized-str"]`):

| Feature | Default | Description |
| --- | :---: | --- |
| `optimized-str` | ✅ | Faster tokenization and paragraph/sentence splitting via the Aho-Corasick algorithm and `memchr::memmem`. Produces identical results to the fallback implementation; disable only to trim dependencies. |
| `optimized-lowercase` | | Faster non-ASCII lowercasing via the `unicode-case-mapping` crate. Requires both this feature *and* a runtime opt-in (`PageAnalysisOptions::optimize_non_ascii`). |
| `python-diff` | | Use the original Python diff algorithm (via `pyo3`) for byte-exact parity with reference WikiWho. Much slower; intended for testing and validation. Also requires a runtime opt-in (`PageAnalysisOptions::use_python_diff`). |
| `strict` | | Make the parser abort on malformed input instead of recovering and continuing. |
| `serde` | | Derive `serde` `Serialize`/`Deserialize` for the public types. **Note:** the serialized `PageAnalysis` format changed in 0.3.0 and is *not* compatible with data produced by earlier versions. |
| `cli` | | Build the `wikiwho-cli` binary for running analysis on dumps from the command line. Implies `serde`. |

The sections below cover the runtime-relevant features in more detail.

### Diff Algorithm Selection

By default, `wikiwho` uses a fast Rust implementation of the histogram diff algorithm (using the `imara-diff` crate). To use the original Python diff algorithm for exact comparison:

```toml
[dependencies]
wikiwho = { version = "0.3", features = ["python-diff"] }
```

and

```rust,ignore
let analysis = PageAnalysis::analyse_page_with_options(&page.revisions, PageAnalysisOptions::new().use_python_diff());
```

**Note**: Using `python-diff` significantly slows down processing as it calls the Python implementation via `pyo3`. This feature is intended for testing and validation purposes. Multi-threading will be less effective because of GIL contention.

### Logging and Error Handling

- Uses the `tracing` crate for logging warnings and errors.
- The parser is designed to recover from errors when possible. Enable the `strict` feature to make the parser terminate upon encountering errors.

```toml
[dependencies]
wikiwho = { version = "0.3", features = ["strict"] }
```

### Optimized String Processing

By default, text splitting functions use straightforward implementations based on `String::replace()` and character iteration. Enable the `optimized-str` feature for faster string processing:

```toml
[dependencies]
wikiwho = { version = "0.3", features = ["optimized-str"] }
```

This swaps in alternative implementations that use the Aho-Corasick algorithm for tokenization and `memchr::memmem` with scratch buffers for paragraph and sentence splitting. These produce identical results and are consistently faster than the default implementations, so enabling this feature is generally recommended (it is included in the default feature set).  
The only case where you might want to disable this feature is if you want to reduce the amount of dependencies.

### Optimized Lowercasing

The `optimized-lowercase` feature replaces the standard library's `str::to_lowercase` with the `unicode-case-mapping` crate. Unlike `optimized-str`, this requires both the cargo feature *and* a runtime opt-in via `PageAnalysisOptions`:

```toml
[dependencies]
wikiwho = { version = "0.3", features = ["optimized-lowercase"] }
```

```rust,ignore
let analysis = PageAnalysis::analyse_page_with_options(
    &page.revisions,
    PageAnalysisOptions::new().optimize_non_ascii(),
);
```

This is only beneficial for text where a significant portion of characters are non-ASCII (roughly less than 90% ASCII). For predominantly ASCII text it is actually *slower* than the stdlib implementation, which has a fast path for ASCII characters. Enable this if you are processing wikis that use scripts with complex Unicode casing rules (e.g., Greek, Armenian, or languages with many diacritics).

## Limitations

- **XML Format Compatibility**: Tested with Wikimedia dump XML format version 0.11. Dumps from other versions or projects may have variations that could cause parsing issues.
- **Accuracy**: By default this crate uses a Rust histogram diff in place of the `difflib` diff used by the original Python WikiWho, so token attributions can differ slightly from that reference implementation on ambiguous tokens. Enable the `python-diff` feature for results byte-identical to Python WikiWho. See [Validation](#validation) for the precision figures.
- **Other Wiki Formats**: Optimized for Wikipedia-like wikis. Users can manually construct `Page` and `Revision` structs from other data sources if needed.

<div class="rustdoc-hidden">

## Contributing

Contributions are welcome — see [`CONTRIBUTING.md`](CONTRIBUTING.md) for ways to help, development setup, the test suite, and CI.

## Security

Releases carry [SLSA build-provenance attestations](https://github.com/actions/attest-build-provenance) and can be independently verified — see [`SECURITY.md`](SECURITY.md) for how to verify a release and how to report a vulnerability.

</div>

## Acknowledgments

This library reimplements the [WikiWho algorithm](https://github.com/wikiwho/WikiWho) originally created by Fabian Flöck and Maribel Acosta. Development was assisted by various AI coding tools.

## Licensing

This project is primarily licensed under the Mozilla Public License 2.0.

However, parts of this project are derived from the
[original `WikiWho` python implementation](https://github.com/wikiwho/WikiWho/), which is licensed
under the MIT License. Thus for these parts of the project (as marked by the SPDX headers) the
MIT License applies additionally.

Generally the MIT license is more permissive than MPL2 though the MIT license terms
and copyright notice must still be preserved.

Wikimedia-derived development fixtures, if present under `dev-data/article-cache/` or
`dev-data/reference-dumps/`, are data rather than code and are documented under those directories'
attribution and licensing notes.
