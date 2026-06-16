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
//!
//! ## Crate layout
//!
//! At a high level the crate is a pipeline of three stages, each in its own module:
//!
//! - **Parse** — [`dump_parser`]: a streaming parser for Wikimedia XML dumps that
//!   yields [`Page`](dump_parser::Page) / [`Revision`](dump_parser::Revision)
//!   values one page at a time.
//! - **Analyse** — [`algorithm`]: the WikiWho authorship algorithm. The entry
//!   point is [`PageAnalysis::analyse_page`](algorithm::PageAnalysis::analyse_page)
//!   (or
//!   [`analyse_page_with_options`](algorithm::PageAnalysis::analyse_page_with_options)
//!   for non-default options).
//! - **Consume** — [`utils`]: helpers for reading results, notably
//!   [`iterate_revision_tokens`](utils::iterate_revision_tokens).
//!
//! Diffing — the heart of the algorithm — lives in two internal modules that are
//! not part of the public API: `difflib` (a clean-room Ratcliff/Obershelp matcher
//! mirroring Python's `difflib`) and `optimized_str` (fast string splitting and
//! lowercasing).

pub mod algorithm;
pub(crate) mod difflib;
pub mod dump_parser;
#[cfg(feature = "optimized-str")]
#[doc(hidden)] /* only public for benchmarking */
pub mod optimized_str;
pub mod utils;
