// SPDX-License-Identifier: MPL-2.0
use std::{collections::HashMap, fmt::Debug};

use compact_str::CompactString;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Contributor {
    pub username: CompactString,
    pub id: Option<i32>,
}

#[derive(Clone, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum Text {
    Normal(String),
    Deleted,
}

impl Text {
    pub fn len(&self) -> usize {
        match self {
            Text::Normal(text) => text.len(),
            Text::Deleted => 0,
        }
    }

    pub fn is_empty(&self) -> bool {
        match self {
            Text::Normal(text) => text.is_empty(),
            Text::Deleted => true,
        }
    }

    /// Returns the text as a string slice.
    ///
    /// If the text is [`Text::Deleted`], an empty string is returned.
    pub fn as_str(&self) -> &str {
        match self {
            Text::Normal(text) => text.as_str(),
            Text::Deleted => "",
        }
    }
}

impl Debug for Text {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Text::Normal(text) => write!(f, "{:?}", text),
            Text::Deleted => write!(f, "Deleted"),
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Sha1Hash(
    /// 31 bytes, sha1 hash -> base36 encoded -> as ASCII bytes
    ///
    /// Simply represents a unique identifier for the text of a revision.
    pub [u8; 31],
);

impl Debug for Sha1Hash {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if let Ok(as_str) = std::str::from_utf8(&self.0) {
            f.debug_tuple("Sha1Hash").field(&as_str).finish()
        } else {
            f.debug_tuple("Sha1Hash").field(&self.0).finish()
        }
    }
}

// apparently `restricted` is never set in mwxml (https://github.com/mediawiki-utilities/python-mwxml/blob/2b477be6aa9794064d03b5be38c7759d1570488b/mwxml/iteration/revision.py#L80)
/// A single revision of a [`Page`].
///
/// When parsing a dump these are produced by the
/// [parser](crate::dump_parser::DumpParser); you can also construct them by hand to
/// feed the [algorithm](crate::algorithm) from a non-MediaWiki source.
///
/// The algorithm only reads a subset of these fields: [`text`](Self::text),
/// [`sha1`](Self::sha1), [`comment`](Self::comment) and [`minor`](Self::minor)
/// affect the analysis, and [`id`](Self::id) is carried through so results can be
/// mapped back to the originating revision. [`timestamp`](Self::timestamp) and
/// [`contributor`](Self::contributor) are preserved for the caller's own
/// bookkeeping but are *not* used by the algorithm — in particular, chronological
/// ordering is taken from the order in which revisions are passed to
/// [`PageAnalysis::analyse_page`](crate::algorithm::PageAnalysis::analyse_page),
/// not from `timestamp`. See the per-field notes for safe defaults when
/// constructing manually.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Revision {
    /// Unique identifier of the revision.
    ///
    /// Carried through analysis unchanged: the origin of every token is reported
    /// as a [`RevisionPointer`](crate::algorithm::RevisionPointer) whose
    /// [`id`](crate::algorithm::RevisionImmutables::id) equals this value, so it
    /// must be unique per revision if you want to map tokens back to their source.
    pub id: i32,
    /// Timestamp of the revision.
    ///
    /// Parser metadata only; not used by the algorithm (which derives order from
    /// the input sequence). When constructing manually any value works, e.g.
    /// `chrono::Utc::now()`.
    pub timestamp: chrono::DateTime<chrono::Utc>,
    /// Author of the revision (aka. user).
    ///
    /// Parser metadata only; not used by the algorithm. To recover authorship, map
    /// a token's origin revision [`id`](Self::id) back to your own data. When
    /// constructing manually, a default/empty [`Contributor`] is fine.
    // aka. user
    pub contributor: Contributor,
    /// Wikitext content of the revision, or [`Text::Deleted`] for revisions whose
    /// text was removed.
    ///
    /// This is the content the algorithm analyses; it is required. Revisions with
    /// [`Text::Deleted`] are skipped.
    pub text: Text,
    /// Optional precomputed SHA-1 of the revision text, as found in the dump.
    ///
    /// Used as a fast content-equality hash during spam/vandalism detection. When
    /// absent, the algorithm hashes the text itself (BLAKE3), so `None` is a safe
    /// default for manual construction.
    pub sha1: Option<Sha1Hash>,
    /// Optional edit summary/comment.
    ///
    /// Together with [`minor`](Self::minor) this tunes one spam-detection
    /// heuristic (a minor edit that carries a comment skips the deletion check).
    /// `None` is a safe default.
    pub comment: Option<CompactString>,
    /// Whether the revision was flagged as a minor edit.
    ///
    /// See [`comment`](Self::comment) for how it is used; `false` is a safe
    /// default.
    pub minor: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Page {
    pub title: CompactString,
    pub namespace: i32,
    pub revisions: Vec<Revision>,
}

#[derive(Clone, PartialEq, Eq, Hash, Default)]
pub enum Namespace {
    #[default]
    Default,
    Named(CompactString),
}

impl Debug for Namespace {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Namespace::Default => write!(f, "Default"),
            Namespace::Named(name) => write!(f, "{:?}", name),
        }
    }
}

#[derive(Debug)]
pub struct SiteInfo {
    pub dbname: CompactString,
    pub namespaces: HashMap<i32, Namespace>,
}
