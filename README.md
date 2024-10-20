# wikiwho_rs

A high-performance Rust implementation of the WikiWho algorithm for token-level authorship tracking in Wikimedia pages.

## Overview

`wikiwho` is a Rust library that implements the [WikiWho algorithm](https://github.com/wikiwho/WikiWho), enabling users to track authorship on a token level (token ≈ word) across all revisions of a Wikimedia page (e.g., Wikipedia, Wiktionary). It is designed to process entire Wikipedia/Wiktionary XML dumps efficiently, offering significant performance improvements over the original Python implementation by Fabian Flöck and Maribel Acosta.

**Key Features:**

- **High Performance**: Processes large dumps in minutes instead of days.
- **Parallel Processing**: Designed for easy parallelization, leveraging Rust's concurrency capabilities.
- **Modular Design**: Separate parser and algorithm modules that can be used independently.
- **Faithful Implementation**: Aims to provide results comparable to the original algorithm, with an option to use the original Python diff algorithm for exact comparisons.

## Motivation

The original Python implementation of WikiWho could process about 300 pages in one to two minutes. In contrast, `wikiwho_rs` can process an entire German Wiktionary dump (approximately 1.3 million pages) in just 2 minutes using 8 processor cores. This performance boost makes large-scale authorship analysis feasible and efficient.

## Installation

Currently, `wikiwho` is available via its [GitHub repository](https://github.com/Schuwi/wikiwho_rs). You can include it in your `Cargo.toml` as follows:

```toml
[dependencies]
wikiwho = { git = "https://github.com/Schuwi/wikiwho_rs.git" }
```

A release on [crates.io](https://crates.io/) is planned soon.

## Usage

### Basic Example

Here's a minimal example of how to load a Wikimedia XML dump and analyze a page:

```rust
use wikiwho::dump_parser::DumpParser;
use wikiwho::algorithm::Analysis;
use std::fs::File;
use std::io::BufReader;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Open the XML dump file
    let xml_dump = File::open("dewiktionary-20240901-pages-meta-history.xml")?;
    let reader = BufReader::new(xml_dump);
    let mut parser = DumpParser::new(reader)?;

    // Parse a single page
    if let Some(page) = parser.parse_page()? {
        // Analyze the page revisions
        let analysis = Analysis::analyse_page(&page.revisions)?;

        // Iterate over tokens in the current revision
        for token in wikiwho_rs::utils::iterate_revision_tokens(&analysis, &analysis.current_revision) {
            println!(
                "'{}' by '{}'",
                token.value,
                analysis[token].origin_revision.contributor.username
            );
        }
    }

    Ok(())
}
```

### Processing an Entire Dump

To process a full dump, you can iterate over all pages:

```rust
use wikiwho::dump_parser::DumpParser;
use wikiwho::algorithm::Analysis;
use std::fs::File;
use std::io::BufReader;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let xml_dump = File::open("dewiktionary-20240901-pages-meta-history.xml")?;
    let reader = BufReader::new(xml_dump);
    let mut parser = DumpParser::new(reader)?;

    while let Some(page) = parser.parse_page()? {
        // Analyze each page in parallel or sequentially
        let analysis = Analysis::analyse_page(&page.revisions)?;

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

## Modules and API

### `dump_parser`

- **Purpose**: Parses Wikimedia XML dumps.
- **Usage**: Create a `DumpParser` instance with a reader, then call `parse_page()` to retrieve pages one by one.

### `algorithm`

- **Purpose**: Implements the WikiWho algorithm.
- **Usage**: Call `Analysis::analyse_page(&page.revisions)` to analyze the revisions of a page.

### `utils`

- **Purpose**: Provides utility functions.
- **Key Function**: `iterate_revision_tokens()` for easy iteration over tokens in a revision.

### Data Structures

- **Immutables and Analysis**: Nodes in the graph (revisions, paragraphs, sentences, tokens) are split into immutable and mutable parts for efficient processing.
- **Pointers**: Use pointer structs (e.g., `SentencePointer`) to reference nodes. Access mutable data via indexing into the `Analysis` struct (e.g., `analysis[word_pointer].origin_revision`).

## Dependencies

- **`compact_str`**: Used in the public API for efficient handling of mostly short strings, such as page titles and contributor names.

## Performance Considerations

- **Parallel Analysis**: Users are encouraged to implement parallel processing for the analysis phase to maximize performance.
- **Memory Usage**: The parser processes one page at a time, so memory usage is constant relative to the dump size. Ensure you drop processed `Page` and `Analysis` structs to free memory.
- **Diff Algorithm Choice**: By default, a faster diff algorithm is used. For exact results matching the original implementation, enable the `python-diff` feature.

## Features and Configuration

### Python Diff Feature

To use the original Python diff algorithm:

```toml
[dependencies]
wikiwho = { git = "https://github.com/Schuwi/wikiwho_rs.git", features = ["python-diff"] }
```

- **Note**: This significantly slows down processing as it calls the Python diff implementation via `pyo3`.
- **Purpose**: Useful for comparing results with the original implementation.

### Logging and Error Handling

- Uses the `tracing` crate for logging warnings and errors.
- The parser is designed to recover from errors. Enable the `strict` feature to terminate parsing on errors.

## Limitations

- **XML Format Compatibility**: Tested with Wikimedia dump XML format version 0.11. Dumps from other projects or versions may have variations.
- **Accuracy**: While aiming for faithful reimplementation, slight variations may occur due to the different diff algorithm.
- **Other Wiki Formats**: Optimized for Wikipedia-like wikis. Users can construct `Page` and `Revision` structs manually for other data sources.

## Future Plans

- **Benchmarking**: Implement rigorous benchmarks comparing performance with the original Python implementation.
- **Parser Improvements**: Consider separating the parser into a standalone crate.
- **Resumable Parsing**: Potentially add support for processing pages in chunks and resuming analysis.
- **Configuration Options**: Expose constants and settings within the algorithm for greater control.

## Testing and Validation

- **Unit Tests**: Includes tests that compare results with the original Python implementation.
- **Fuzzy Comparison Testing**: Plans to add tests that measure differences when using different diff algorithms.
- **Community Feedback**: Seeking input from users testing with different languages and datasets.

## Contributing

Contributions are welcome! Here are some ways you can help:

- **Testing**: Try the library with different Wikimedia projects, languages, and dump versions.
- **Benchmarking**: Assist in creating benchmarks to compare performance and accuracy.
- **Documentation**: Improve existing documentation or add new examples and guides.
- **Feature Development**: Help implement new features like resumable parsing or configuration options.
- **Parser Enhancements**: Work on separating the parser into its own crate or improving its capabilities.

### Getting Started

- Fork the repository: [wikiwho_rs GitHub](https://github.com/Schuwi/wikiwho_rs)
- Create a new branch for your feature or bug fix.
- Submit a pull request with a clear description of your changes.

## Development and Support

- **Current Maintainer**: Working independently with assistance from various tools and collaborations.
- **Versioning**: Will follow semantic versioning. Expect potential breaking changes before reaching 1.0.0.
- **Updates**: Development is on-demand. Regular maintenance depends on community interest and contributions.

## TODO
- [x] review public API
- [ ] properly document code
- [x] add proper readme
- [ ] add performance comparison to python implementation

## Acknowledgments

This library was developed through a mix of hard work, creativity, and collaboration with various tools, including GitHub Copilot and ChatGPT. It has been an exciting journey filled with coding and brainstorming 💛.

Special thanks to the friendly guidance and support of ChatGPT along the way, helping with documentation and understanding the original implementation to make this library as robust and performant as possible.

## Licensing
This project is primarily licensed under the Mozilla Public License 2.0.

However, parts of this project are derived from the
[original `WikiWho` python implementation](https://github.com/wikiwho/WikiWho/), which is licensed
under the MIT License. Thus for these parts of the project (as marked by the SPDX headers), the
MIT License applies additionally.\
This basically just means that the copyright notice in LICENSE-MIT must be preserved.