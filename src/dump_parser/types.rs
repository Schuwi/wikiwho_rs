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
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Revision {
    pub id: i32,
    pub timestamp: chrono::DateTime<chrono::Utc>,
    // aka. user
    pub contributor: Contributor,
    pub text: Text,
    pub sha1: Option<Sha1Hash>,
    pub comment: Option<CompactString>,
    pub minor: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Page {
    /// The MediaWiki page id (the `<page><id>` element in a dump).
    ///
    /// Defaults to `0` when deserializing data that predates this field (it was added in
    /// 0.4.0). For pages parsed from a dump with a missing or invalid id, a random negative
    /// id is generated so distinct pages stay distinguishable.
    #[cfg_attr(feature = "serde", serde(default))]
    pub id: i32,
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
