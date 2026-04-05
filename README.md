<div class="rustdoc-hidden">

# wikiwho

</div>

A high-performance Rust implementation of the WikiWho algorithm for token-level authorship tracking in Wikimedia pages.

## Overview

`wikiwho` is a Rust library that implements the [WikiWho algorithm](https://github.com/wikiwho/WikiWho), enabling users to track authorship on a token level (token ≈ word) across all revisions of a Wikimedia page (e.g., Wikipedia, Wiktionary). It reimplements the original algorithm by Fabian Flöck and Maribel Acosta with significant performance improvements, allowing for efficient processing of entire Wikipedia/Wiktionary XML dumps.

**Key Features:**

- **High Performance**: Processes large dumps in minutes instead of days.
- **Parallel Processing**: Designed for easy parallelization, leveraging Rust's concurrency capabilities.
- **Modular Design**: Separate parser and algorithm modules that can be used independently.
- **Faithful Implementation**: Aims to provide results comparable to the original algorithm, with an option to use the original Python diff algorithm for exact comparisons.

<div class="rustdoc-hidden">

## Motivation

The original Python implementation of WikiWho could process about 300 pages in one to two minutes. In contrast, `wikiwho_rs` can process an entire German Wiktionary dump (approximately 1.3 million pages) in just under 4 minutes using 8 processor cores. This performance boost makes large-scale authorship analysis feasible and efficient.

</div>

## Installation

`wikiwho` is available on [crates.io](https://crates.io/crates/wikiwho). Add it to your `Cargo.toml`:

```toml
[dependencies]
wikiwho = "0.2"
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
wikiwho = { version = "0.2", features = ["python-diff"] }
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
wikiwho = { version = "0.2", features = ["strict"] }
```

### Optimized String Processing

By default, text splitting functions use straightforward implementations based on `String::replace()` and character iteration. Enable the `optimized-str` feature for faster string processing:

```toml
[dependencies]
wikiwho = { version = "0.2", features = ["optimized-str"] }
```

This swaps in alternative implementations that use the Aho-Corasick algorithm for tokenization and `memchr::memmem` with scratch buffers for paragraph and sentence splitting. These produce identical results and are consistently faster than the default implementations, so enabling this feature is generally recommended (it is included in the default feature set).  
The only case where you might want to disable this feature is if you want to reduce the amount of dependencies.

### Optimized Lowercasing

The `optimized-lowercase` feature replaces the standard library's `str::to_lowercase` with the `unicode-case-mapping` crate. Unlike `optimized-str`, this requires both the cargo feature *and* a runtime opt-in via `PageAnalysisOptions`:

```toml
[dependencies]
wikiwho = { version = "0.2", features = ["optimized-lowercase"] }
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

## Future Plans

- **Benchmarking**: Implement rigorous benchmarks comparing performance with the original Python implementation.
- **Parser Improvements**: Consider separating the parser into a standalone crate.
- **Resumable Parsing**: Potentially add support for processing pages in chunks and resuming analysis.
- **Configuration Options**: Expose constants and settings within the algorithm for greater control.

## Testing and Validation

- **Exact comparison tests** (`algorithm_exact_tests.rs`): Compare the Rust implementation's results against the original Python WikiWho, token by token. These require the `python-diff` feature so that both implementations use the same diff algorithm. Run them with `cargo test --features python-diff`.
- **Statistical comparison tests** (`algorithm_statistic_tests.rs`): Ignored by default and require local benchmark data. Fetch the archived partial gold standard with `python3 tools/fetch_gold_standard.py`, place current Wikimedia dump shards into `dev-data/extra-dumps/`, then run with `cargo test gold_standard_precision_rust -- --ignored` or `cargo test --features python-diff divergence_rate_gold_standard_articles -- --ignored`. See `dev-data/README.md` for details.
- **Temporary files**: Some tests use temporary files for IPC coordination between Rust and Python. These files can be large depending on the input dump. Their location follows `std::env::temp_dir()`, which can be controlled by setting the `TMPDIR` environment variable.
- **Community Feedback**: Seeking input from users testing with different languages and datasets.

## Contributing

Contributions are welcome! Here are some ways you can help:

- **Testing**: Try the library with different Wikimedia projects, languages, and dump versions.
- **Benchmarking**: Assist in creating benchmarks to compare performance and accuracy.
- **Documentation**: Improve existing documentation or add new examples and guides.
- **Feature Development**: Help implement new features like resumable parsing or configuration options.
- **Parser Enhancements**: Work on separating the parser into its own crate or improving its capabilities.

By submitting a contribution, you agree that your code will be licensed under this project’s license.

### Getting Started

- Fork the repository: [wikiwho_rs GitHub](https://github.com/Schuwi/wikiwho_rs)
- Create a new branch for your feature or bug fix.
- Submit a pull request with a clear description of your changes.

### Development Setup

The exact comparison tests call into the original Python WikiWho implementation to validate results, so a Python virtual environment must be active when running them. Without it, tests will fail with cryptic Python/pyo3 errors.

```sh
python -m venv venv
source venv/bin/activate   # on Windows: venv\Scripts\activate
pip install -r requirements.txt
cargo test --features python-diff
```

To control where large temporary IPC files are written, set `TMPDIR` before running:

```sh
TMPDIR=/path/with/space cargo test --features python-diff
```

## Development and Support

- **Current Maintainer**: Working independently with assistance from various tools and collaborations.
- **Versioning**: Will follow semantic versioning. Expect potential breaking changes before reaching 1.0.0.
- **Updates**: Development is on-demand. Regular maintenance depends on community interest and contributions.

</div>

## Acknowledgments

This library was developed through a mix of hard work, creativity, and collaboration with various tools, including GitHub Copilot, ChatGPT and Claude Code. It has been an exciting journey filled with coding and brainstorming 💛.

Special thanks to the friendly guidance and support of ChatGPT along the way, helping with documentation and understanding the original implementation to make this library as robust and performant as possible.

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
