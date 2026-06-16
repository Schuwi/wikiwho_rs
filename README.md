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

## Usage

### Basic Example

Here's a minimal example of how to load a Wikimedia XML dump and analyze a page:

```rust,no_run
use wikiwho::dump_parser::{DumpParser, Revision};
use wikiwho::algorithm::PageAnalysis;
use std::collections::HashMap;
use std::fs::File;
use std::io::BufReader;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let xml_dump = File::open("path/to/pages-meta-history.xml")?;
    let reader = BufReader::new(xml_dump);
    let mut parser = DumpParser::new(reader)?;

    // Parse a single page
    if let Some(page) = parser.parse_page()? {
        // Analyze the page revisions
        let analysis = PageAnalysis::analyse_page(&page.revisions)?;

        let revisions_by_id: HashMap<i32, Revision> = page.revisions.into_iter()
            .map(|rev| (rev.id, rev))
            .collect();

        // Iterate over tokens in the current revision
        for token in wikiwho::utils::iterate_revision_tokens(&analysis, &analysis.current_revision) {
            let token_analysis = &analysis[token];
            let origin_revision_xml = &revisions_by_id[&token_analysis.origin_revision.id];
            println!(
                "'{}' by '{}'",
                token.value.as_str(),
                origin_revision_xml.contributor.username
            );
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

While XML parsing is inherently linear, you can process pages in parallel once they are parsed:

- Run the parser in a single thread.
- Distribute parsed pages to worker threads for analysis.
- Use threading libraries like `std::thread` or crates like `rayon` for concurrency.

**Example using multiple threads:**

```rust,no_run
use wikiwho::dump_parser::{DumpParser, Page};
use wikiwho::algorithm::PageAnalysis;
use std::fs::File;
use std::io::BufReader;
use std::sync::{mpsc::channel, Arc, Mutex};
use std::thread;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let xml_dump = File::open("path/to/pages-meta-history.xml")?;
    let reader = BufReader::new(xml_dump);
    let mut parser = DumpParser::new(reader)?;

    // Channel to send pages to worker threads
    let (tx, rx) = channel::<Page>();
    let rx = Arc::new(Mutex::new(rx));

    // Spawn worker threads
    let num_workers = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1);
    let workers: Vec<_> = (0..num_workers)
        .map(|_| {
            let rx = Arc::clone(&rx);
            thread::spawn(move || {
                loop {
                    let page = rx.lock().unwrap().recv();
                    match page {
                        Ok(page) => {
                            // Analyze the page
                            let analysis = PageAnalysis::analyse_page(&page.revisions).unwrap();
                            // Processing logic
                        }
                        Err(_) => break,
                    }
                }
            })
        })
        .collect();

    // Parse pages and send them to workers
    while let Some(page) = parser.parse_page()? {
        tx.send(page)?;
    }
    drop(tx); // Close the channel

    // Wait for all workers to finish
    for worker in workers {
        worker.join().unwrap();
    }

    Ok(())
}
```

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

## Dependencies

- **`compact_str`**: Used in the public API for efficient handling of mostly short strings, such as page titles and contributor names.

## Performance Considerations

- **Parallel Analysis**: Users are encouraged to implement parallel processing for the analysis phase to maximize performance.
- **Parsing Bottleneck**: XML parsing is linear and may become a bottleneck. Running the parser in a single thread and distributing analysis can optimize performance.
- **Memory Usage**: The parser processes one page at a time, so memory usage is constant relative to the dump size. Ensure you drop processed `Page` and `PageAnalysis` structs to free memory.
- **Diff Algorithm Choice**: By default, a faster diff algorithm is used. For exact results matching the original implementation, enable the `python-diff` feature and use `PageAnalysis::analyse_page_with_options` to select the Python diff algorithm.

## Features and Configuration

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
- **Accuracy**: While the library aims for a faithful reimplementation, slight variations may occur due to differences in the diff algorithm.
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
