// SPDX-License-Identifier: MPL-2.0
//! # wikiwho
//!
//! A high-performance Rust implementation of the WikiWho algorithm for token-level authorship tracking in Wikimedia pages.
//!
//! ## Overview
//!
//! `wikiwho` is a Rust library that implements the [WikiWho algorithm](https://github.com/wikiwho/WikiWho), enabling users to track authorship on a token level (token ≈ word) across all revisions of a Wikimedia page (e.g., Wikipedia, Wiktionary). It reimplements the original algorithm by Fabian Flöck and Maribel Acosta with significant performance improvements, allowing for efficient processing of entire Wikipedia/Wiktionary XML dumps.
//!
//! **Key Features:**
//!
//! - **High Performance**: Processes large dumps in minutes instead of days.
//! - **Parallel Processing**: Designed for easy parallelization, leveraging Rust's concurrency capabilities.
//! - **Modular Design**: Separate parser and algorithm modules that can be used independently.
//! - **Faithful Implementation**: Aims to provide results comparable to the original algorithm, with an option to use the original Python diff algorithm for exact comparisons.
//!
//! ## Getting Started
//!
//! ### Installation
//!
//! Add `wikiwho` to your `Cargo.toml`:
//!
//! ```toml
//! [dependencies]
//! wikiwho = "0.1.0"  # Update with the actual version once released
//! ```
//!
//! ### Basic Usage
//!
//! Here's a minimal example of how to load a Wikimedia XML dump and analyze a page:
//!
//! ```rust
//! use wikiwho::dump_parser::DumpParser;
//! use wikiwho::algorithm::Analysis;
//! use std::fs::File;
//! use std::io::BufReader;
//!
//! fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     // Open the XML dump file
//!     let xml_dump = File::open("dewiktionary-20240901-pages-meta-history.xml")?;
//!     let reader = BufReader::new(xml_dump);
//!     let mut parser = DumpParser::new(reader)?;
//!
//!     // Parse a single page
//!     if let Some(page) = parser.parse_page()? {
//!         // Analyze the page revisions
//!         let analysis = Analysis::analyse_page(&page.revisions)?;
//!
//!         // Iterate over tokens in the current revision
//!         for token in wikiwho::utils::iterate_revision_tokens(&analysis, &analysis.current_revision) {
//!             println!(
//!                 "'{}' by '{}'",
//!                 token.value,
//!                 analysis[token].origin_revision.contributor.username
//!             );
//!         }
//!     }
//!
//!     Ok(())
//! }
//! ```
//!
//! ### Processing an Entire Dump
//!
//! To process a full dump, you can iterate over all pages:
//!
//! ```rust
//! use wikiwho::dump_parser::DumpParser;
//! use wikiwho::algorithm::Analysis;
//! use std::fs::File;
//! use std::io::BufReader;
//!
//! fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     let xml_dump = File::open("dewiktionary-20240901-pages-meta-history.xml")?;
//!     let reader = BufReader::new(xml_dump);
//!     let mut parser = DumpParser::new(reader)?;
//!
//!     while let Some(page) = parser.parse_page()? {
//!         // Analyze each page (can be parallelized)
//!         let analysis = Analysis::analyse_page(&page.revisions)?;
//!
//!         // Your processing logic here
//!     }
//!
//!     Ok(())
//! }
//! ```
//!
//! ### Parallel Processing
//!
//! While XML parsing is inherently linear, you can process pages in parallel once they are parsed:
//!
//! - Run the parser in a single thread.
//! - Distribute parsed pages to worker threads for analysis.
//! - Utilize threading libraries like `std::thread` or concurrency crates like `rayon`.
//!
//! **Example using multiple threads:**
//!
//! ```rust
//! use wikiwho::dump_parser::DumpParser;
//! use wikiwho::algorithm::Analysis;
//! use std::fs::File;
//! use std::io::BufReader;
//! use std::sync::mpsc::channel;
//! use std::thread;
//!
//! fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     let xml_dump = File::open("dewiktionary-20240901-pages-meta-history.xml")?;
//!     let reader = BufReader::new(xml_dump);
//!     let mut parser = DumpParser::new(reader)?;
//!
//!     // Channel to send pages to worker threads
//!     let (tx, rx) = channel();
//!
//!     // Spawn worker threads
//!     let workers: Vec<_> = (0..num_cpus::get()-1)
//!         .map(|_| {
//!             let rx = rx.clone();
//!             thread::spawn(move || {
//!                 for page in rx.iter() {
//!                     // Analyze the page
//!                     let analysis = Analysis::analyse_page(&page.revisions).unwrap();
//!                     // Processing logic
//!                 }
//!             })
//!         })
//!         .collect();
//!
//!     // Parse pages and send them to workers
//!     while let Some(page) = parser.parse_page()? {
//!         tx.send(page)?;
//!     }
//!     drop(tx); // Close the channel
//!
//!     // Wait for all workers to finish
//!     for worker in workers {
//!         worker.join().unwrap();
//!     }
//!
//!     Ok(())
//! }
//! ```
//!
//! ## Modules and API
//!
//! ### `dump_parser` Module
//!
//! **Purpose**: Parses Wikimedia XML dumps.
//!
//! **Usage**:
//!
//! - Create a `DumpParser` instance with a reader.
//! - Use `parse_page()` to retrieve pages one by one.
//! - Access site information using `dump_parser.site_info()`.
//!
//! **Example**:
//!
//! ```rust
//! let xml_dump = File::open("path_to_dump.xml")?;
//! let reader = BufReader::new(xml_dump);
//! let mut parser = DumpParser::new(reader)?;
//!
//! while let Some(page) = parser.parse_page()? {
//!     // Process the page
//! }
//! ```
//!
//! ### `algorithm` Module
//!
//! **Purpose**: Implements the WikiWho algorithm.
//!
//! **Usage**:
//!
//! - Call `Analysis::analyse_page(&page.revisions)` to analyze the revisions of a page.
//! - Access the analysis results via the `Analysis` struct.
//!
//! **Example**:
//!
//! ```rust
//! let analysis = Analysis::analyse_page(&page.revisions)?;
//! ```
//!
//! ### `utils` Module
//!
//! **Purpose**: Provides utility functions.
//!
//! **Key Function**: `iterate_revision_tokens()` for easy iteration over tokens in a revision.
//!
//! **Example**:
//!
//! ```rust
//! for token_pointer in wikiwho::utils::iterate_revision_tokens(&analysis, &analysis.current_revision) {
//!     // Use the token pointer to access token data
//!     let token = &analysis[token_pointer];
//!
//!     // Use the token
//! }
//! ```
//!
//! ## Data Structures
//!
//! The library uses a graph structure to represent the relationships between revisions, paragraphs, sentences, and tokens. To efficiently manage this, each node is split into two parts:
//!
//! - **Immutable Part**: Contains data that doesn't change during analysis (e.g., the text of a token).
//! - **Mutable Part**: Contains data that is updated during analysis (e.g., the origin of a token).
//!
//! **Pointers and Indexing**
//!
//! - Nodes are referenced using pointer structs (e.g., `SentencePointer`), which include an index and a shared reference to the immutable data.
//! - To access mutable data, use indexing into the `Analysis` struct:
//!
//! ```rust
//! let origin_revision = &analysis[word_pointer].origin_revision;
//! ```
//!
//! - Alternatively you may index into the corresponding `Vec` in the `Analysis` struct directly:
//!
//! ```rust
//! let origin_revision = &analysis.words[word_pointer.0].origin_revision;
//! ```
//!
//! ## Performance Considerations
//!
//! - **Parsing Bottleneck**: XML parsing is linear and may become a bottleneck. Running the parser in a single thread and distributing analysis can optimize performance.
//! - **Memory Usage**: The parser processes one page at a time. Memory usage should remain constant relative to the dump size. Ensure you drop `Page` and `Analysis` structs after processing.
//! - **Scalability**: The analysis phase is designed for parallel execution. Utilize multiple threads to process pages concurrently after parsing.
//!
//! ## Features and Configuration
//!
//! ### Diff Algorithm Selection
//!
//! By default, `wikiwho` uses a fast Rust implementation of the histogram diff algorithm (using the `imara-diff` crate). To use the original Python diff algorithm for exact comparison:
//!
//! ```toml
//! [dependencies]
//! wikiwho = { version = "0.1.0", features = ["python-diff"] }
//! ```
//!
//! **Note**: Enabling `python-diff` significantly slows down processing as it calls the Python implementation via `pyo3`. This feature is intended for testing and validation purposes.
//!
//! ### Logging and Error Handling
//!
//! - Uses the `tracing` crate for logging warnings and errors.
//! - The parser is designed to recover from errors when possible. Enable the `strict` feature to make the parser terminate upon encountering errors.
//!
//! **Example**:
//!
//! ```toml
//! [dependencies]
//! wikiwho = { version = "0.1.0", features = ["strict"] }
//! ```
//!
//! ## Limitations
//!
//! - **XML Format Compatibility**: Tested with Wikimedia dump XML format version 0.11. Dumps from other versions or projects may have variations that could cause parsing issues.
//! - **Accuracy**: While the library aims for a faithful reimplementation, slight variations will occur due to differences in the diff algorithm.
//! - **Other Wiki Formats**: The library is optimized for Wikipedia-like wikis. Users can manually construct `Page` and `Revision` structs from other data sources if needed.
//!
//! ## Dependencies
//!
//! - **`compact_str`**: Used in the public API for efficient handling of short strings (e.g., page titles, contributor names).
//!
//! ## Licensing
//!
//! This project is primarily licensed under the Mozilla Public License 2.0.
//!
//! However, parts of this project are derived from the [original `WikiWho` Python implementation](https://github.com/wikiwho/WikiWho/), which is licensed under the MIT License. For these parts of the project (as marked by the SPDX headers), the MIT License applies additionally. This means that the copyright notice in `LICENSE-MIT` must be preserved.
//!
//! ## Acknowledgments
//!
//! This library was developed through a mix of hard work, creativity, and collaboration with various tools, including GitHub Copilot and ChatGPT. It has been an exciting journey filled with coding and brainstorming 💛.
//!
//! Special thanks to the friendly guidance and support of ChatGPT, which assisted in documentation and brainstorming ideas to make this library as robust and performant as possible.

pub mod algorithm;
pub mod dump_parser;
// it only makes sense to compare the algorithm to python if the same diff algorithm is used
#[cfg(all(test, feature = "python-diff"))]
mod integration_tests;
#[cfg(test)]
mod test_support;
pub mod utils;
