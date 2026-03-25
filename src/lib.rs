// SPDX-License-Identifier: MPL-2.0
//! <style>
//! .rustdoc-hidden { display: none; }
//! </style>
#![doc = include_str!("../README.md")]
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
//! - To access mutable data, use indexing into the `PageAnalysis` struct:
//!
//! ```rust,ignore
//! let origin_revision = &analysis[word_pointer].origin_revision;
//! ```
//!
//! - Alternatively you may access the `words` field on `PageAnalysis` directly:
//!
//! ```rust,ignore
//! let origin_revision = &analysis.words[word_pointer.unique_id()].origin_revision;
//! ```

pub mod algorithm;
pub mod dump_parser;
#[cfg(feature = "optimized-str")]
#[doc(hidden)] /* only public for benchmarking */
pub mod optimized_str;
pub mod utils;
