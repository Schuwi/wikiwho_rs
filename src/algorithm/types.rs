// SPDX-License-Identifier: MIT AND MPL-2.0
use chrono::prelude::*;
use compact_str::CompactString;
use rustc_hash::FxHashMap;
use std::{
    collections::HashMap,
    fmt::Debug,
    ops::{Deref, Index, IndexMut},
    sync::Arc,
};

use crate::{
    dump_parser::{Contributor, Revision, Text},
    utils,
};

use super::PageAnalysisInternals;

struct StringLenLimited<'a>(&'a str, usize);

impl Debug for StringLenLimited<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.0.len() > self.1 {
            let split = self
                .0
                .char_indices()
                .nth(self.1)
                .map(|(idx, _)| idx)
                .unwrap_or(self.0.len());
            write!(f, "{}...", &self.0[..split])
        } else {
            write!(f, "{}", self.0)
        }
    }
}

/// A container that holds either a single element or a `Vec`.
///
/// Memory optimization for hash-map values where the common case is exactly
/// one entry (e.g., paragraph and sentence lookups by hash). Avoids a heap
/// allocation for the single-element case.
#[derive(Clone)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum MaybeVec<T> {
    Single(T),
    Vec(Vec<T>),
}

impl<T> MaybeVec<T> {
    pub fn new_single(value: T) -> Self {
        MaybeVec::Single(value)
    }

    pub fn new_vec(value: Vec<T>) -> Self {
        MaybeVec::Vec(value)
    }

    pub fn as_slice(&self) -> &[T] {
        match self {
            MaybeVec::Single(t) => std::slice::from_ref(t),
            MaybeVec::Vec(v) => v,
        }
    }

    pub fn push(&mut self, value: T) {
        let mut temp = MaybeVec::new_vec(Vec::new());
        std::mem::swap(&mut temp, self);

        match temp {
            MaybeVec::Single(t) => {
                let vec = vec![t, value];
                *self = MaybeVec::Vec(vec);
            }
            MaybeVec::Vec(mut v) => {
                v.push(value);
                *self = MaybeVec::Vec(v);
            }
        }
    }

    pub fn into_vec(self) -> Vec<T> {
        match self {
            MaybeVec::Single(t) => vec![t],
            MaybeVec::Vec(v) => v,
        }
    }

    pub fn len(&self) -> usize {
        match self {
            MaybeVec::Single(_) => 1,
            MaybeVec::Vec(v) => v.len(),
        }
    }

    pub fn is_empty(&self) -> bool {
        match self {
            MaybeVec::Single(_) => false,
            MaybeVec::Vec(v) => v.is_empty(),
        }
    }
}

// index is unique within a page
#[derive(Clone)]
pub struct RevisionPointer(pub(crate) usize, pub(crate) Arc<RevisionImmutables>);

impl Deref for RevisionPointer {
    type Target = RevisionImmutables;

    fn deref(&self) -> &Self::Target {
        &self.1
    }
}

impl Debug for RevisionPointer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "RevisionPointer({}, '{:?}')",
            self.0,
            StringLenLimited(&self.text_lowercase, 20)
        )
    }
}

impl PartialEq for RevisionPointer {
    fn eq(&self, other: &Self) -> bool {
        self.0 == other.0
    }
}

impl Eq for RevisionPointer {}

#[derive(Clone)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct RevisionImmutables {
    pub length_lowercase: usize, /* text length when lowercased, in bytes (for `test` compile target this is the number of unicode codepoints, to match the python implementation) */
    pub text_lowercase: String,  /* lowercased text of revision */
    pub xml_revision: Revision,
}

impl RevisionImmutables {
    pub fn dummy() -> Self {
        Self {
            length_lowercase: 0,
            text_lowercase: String::new(),
            xml_revision: Revision {
                id: 0,
                timestamp: Utc::now(),
                contributor: Contributor {
                    id: None,
                    username: CompactString::new(""),
                },
                comment: None,
                minor: false,
                text: Text::Normal(String::new()),
                sha1: None,
            },
        }
    }

    pub fn from_revision(revision: &Revision) -> Self {
        Self {
            // #[cfg(not(any(test, feature = "match-reference-impl")))]
            // // for spam detection it should be enough to use the length of the text in bytes
            // length: revision.text.len(),
            // #[cfg(any(test, feature = "match-reference-impl"))]
            // python's `len` function returns the number of unicode codepoints for a string,
            // so when testing against the python implementation we need to match that behavior to get identical results
            length_lowercase: revision.text.as_str().chars().count(),
            text_lowercase: match revision.text {
                Text::Normal(ref t) => utils::to_lowercase(t),
                Text::Deleted => String::new(),
            },
            xml_revision: revision.clone(),
        }
    }
}

impl Deref for RevisionImmutables {
    type Target = Revision;

    fn deref(&self) -> &Self::Target {
        &self.xml_revision
    }
}

#[derive(Clone, Default)]
pub struct RevisionAnalysis {
    pub(crate) paragraphs_by_hash: FxHashMap<blake3::Hash, MaybeVec<ParagraphPointer>>, /* assume that duplicate paragraphs are not very common and optimize to avoid allocation */
    pub paragraphs_ordered: Vec<ParagraphPointer>,

    pub original_adds: usize, /* number of tokens added in this revision for the first time */
}

// index is unique within a page
#[derive(Clone)]
pub struct ParagraphPointer(pub(crate) usize, pub(crate) Arc<ParagraphImmutables>);

impl Deref for ParagraphPointer {
    type Target = ParagraphImmutables;

    fn deref(&self) -> &Self::Target {
        &self.1
    }
}

impl Debug for ParagraphPointer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "ParagraphPointer({}, '{:?}')",
            self.0,
            StringLenLimited(&self.value, 20)
        )
    }
}

#[derive(Clone)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct ParagraphImmutables {
    #[cfg_attr(feature = "serde", serde(skip))]
    pub(crate) hash_value: blake3::Hash,
    pub value: String,
}

#[cfg(feature = "serde")]
impl<'de> serde::Deserialize<'de> for ParagraphImmutables {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(serde::Deserialize)]
        struct Helper {
            value: String,
        }
        let helper = Helper::deserialize(deserializer)?;
        Ok(Self::new(helper.value))
    }
}

impl ParagraphImmutables {
    pub fn new(value: String) -> Self {
        let hash_value = blake3::hash(value.as_bytes());
        Self { hash_value, value }
    }

    pub fn hash(&self) -> &[u8] {
        /* return a slice of bytes as not to commit our API to any hash algorithm */
        self.hash_value.as_bytes()
    }
}

#[derive(Clone, Default)]
pub struct ParagraphAnalysis {
    pub(crate) sentences_by_hash: FxHashMap<blake3::Hash, MaybeVec<SentencePointer>>,
    pub sentences_ordered: Vec<SentencePointer>,

    /// whether this paragraph was found in the current revision
    pub(crate) matched_in_current: bool,
}

// index is unique within a page
#[derive(Clone)]
pub struct SentencePointer(pub(crate) usize, pub(crate) Arc<SentenceImmutables>);

impl Deref for SentencePointer {
    type Target = SentenceImmutables;

    fn deref(&self) -> &Self::Target {
        &self.1
    }
}

impl Debug for SentencePointer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "SentencePointer({}, '{:?}')",
            self.0,
            StringLenLimited(&self.value, 20)
        )
    }
}

#[derive(Clone)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct SentenceImmutables {
    #[cfg_attr(feature = "serde", serde(skip))]
    pub(crate) hash_value: blake3::Hash,
    pub value: String,
}

#[cfg(feature = "serde")]
impl<'de> serde::Deserialize<'de> for SentenceImmutables {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(serde::Deserialize)]
        struct Helper {
            value: String,
        }
        let helper = Helper::deserialize(deserializer)?;
        Ok(Self::new(helper.value))
    }
}

impl SentenceImmutables {
    pub fn new(value: String) -> Self {
        let hash_value = blake3::hash(value.as_bytes());
        Self { hash_value, value }
    }

    pub fn hash(&self) -> &[u8] {
        self.hash_value.as_bytes()
    }
}

#[derive(Clone, Default)]
pub struct SentenceAnalysis {
    pub words_ordered: Vec<WordPointer>,

    /// whether this sentence was found in the current revision
    pub(crate) matched_in_current: bool,
}

// index is unique within a page
#[derive(Clone)]
pub struct WordPointer(pub(crate) usize, pub(crate) Arc<WordImmutables>);

impl WordPointer {
    pub fn unique_id(&self) -> usize {
        self.0
    }
}

impl Deref for WordPointer {
    type Target = WordImmutables;

    fn deref(&self) -> &Self::Target {
        &self.1
    }
}

impl Debug for WordPointer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "WordPointer({}, '{:?}')",
            self.0,
            StringLenLimited(&self.1.value, 20)
        )
    }
}

#[derive(Clone)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct WordImmutables {
    pub value: CompactString,
}

impl WordImmutables {
    pub fn new(value: CompactString) -> Self {
        Self { value }
    }
}

/// Mutable analysis data tracked for each word (token) across revisions.
#[derive(Clone)]
pub struct WordAnalysis {
    /// The revision that first introduced this word. Set once and never changed.
    pub origin_revision: RevisionPointer,
    /// The most recent revision in which this word was present (matched).
    /// Updated each time the word is found in a new revision.
    pub latest_revision: RevisionPointer,
    /// Whether this word was found in the revision currently being analysed.
    pub(crate) matched_in_current: bool,

    /// Revisions where this word was re-added after having been removed.
    ///
    /// Each entry records a revision in which the word reappeared after being
    /// absent in the preceding (non-spam) revision.
    pub inbound: Vec<RevisionPointer>,
    /// Revisions where this word was removed.
    ///
    /// Each entry records a revision in which the word was absent after being
    /// present in the preceding revision.
    pub outbound: Vec<RevisionPointer>,
}

impl WordAnalysis {
    pub fn new(origin_rev: &RevisionPointer) -> Self {
        Self {
            origin_revision: origin_rev.clone(),
            latest_revision: origin_rev.clone(),
            matched_in_current: false,
            inbound: Vec::new(),
            outbound: Vec::new(),
        }
    }
}

pub struct PageAnalysis {
    // single array where the structural and analytical information of all the revisions/paragraphs/sentences/words in this page is stored
    // the goal is to work with Rust's memory model and avoid falling back to Arc<RefCell<...>> everywhere
    pub(crate) revisions: Vec<RevisionAnalysis>, /* access via ordered_revisions */
    pub(crate) revision_immutables: Vec<Arc<RevisionImmutables>>, // lockstep with revisions
    pub(crate) paragraphs: Vec<ParagraphAnalysis>,
    pub(crate) paragraph_immutables: Vec<Arc<ParagraphImmutables>>, // lockstep with paragraphs
    pub(crate) sentences: Vec<SentenceAnalysis>,
    pub(crate) sentence_immutables: Vec<Arc<SentenceImmutables>>, // lockstep with sentences
    pub(crate) word_analyses: Vec<WordAnalysis>,
    pub(crate) word_immutables: Vec<Arc<WordImmutables>>, // lockstep with words

    /// Collection of revision IDs that were detected as spam.
    ///
    /// These revisions were not analysed and are not included in the `revisions`,
    /// `revisions_by_id` and `ordered_revisions` fields.
    pub spam_ids: Vec<i32>,
    /// Map of revision ID to RevisionData.
    ///
    /// Does not contain revisions that were detected as spam.
    pub revisions_by_id: HashMap<i32, RevisionPointer>,
    /// List of revisions in order from oldest to newest.
    ///
    /// Does not contain revisions that were detected as spam.
    pub ordered_revisions: Vec<RevisionPointer>,
    /// Ordered, unique list of tokens in the page
    pub words: Vec<WordPointer>,

    /// The current revision being analysed.
    ///
    /// After analysis finished this will be the latest revision that was not marked as spam.
    pub current_revision: RevisionPointer,

    pub(crate) internals: PageAnalysisInternals,
}

impl PageAnalysis {
    /// Creates a PageAnalysis initialized with the given revision as `current_revision`.
    /// For normal use, prefer `analyse_page()`.
    pub fn new(initial_revision: (RevisionAnalysis, RevisionImmutables)) -> Self {
        let arc = Arc::new(initial_revision.1);
        let initial_revision_ptr = RevisionPointer(0, arc.clone());

        Self {
            revisions: vec![initial_revision.0],
            revision_immutables: vec![arc],
            paragraphs: Vec::new(),
            paragraph_immutables: Vec::new(),
            sentences: Vec::new(),
            sentence_immutables: Vec::new(),
            word_analyses: Vec::new(),
            word_immutables: Vec::new(),
            spam_ids: Vec::new(),
            revisions_by_id: HashMap::new(),
            ordered_revisions: Vec::new(),
            words: Vec::new(),
            current_revision: initial_revision_ptr,
            internals: PageAnalysisInternals::default(),
        }
    }

    pub fn new_revision(&mut self, revision_data: RevisionImmutables) -> RevisionPointer {
        let arc = Arc::new(revision_data);
        let revision_pointer = RevisionPointer(self.revisions.len(), arc.clone());
        self.revisions.push(RevisionAnalysis::default());
        self.revision_immutables.push(arc);
        revision_pointer
    }

    pub fn new_paragraph(&mut self, paragraph_data: ParagraphImmutables) -> ParagraphPointer {
        let arc = Arc::new(paragraph_data);
        let paragraph_pointer = ParagraphPointer(self.paragraphs.len(), arc.clone());
        self.paragraphs.push(ParagraphAnalysis::default());
        self.paragraph_immutables.push(arc);
        paragraph_pointer
    }

    pub fn new_sentence(&mut self, sentence_data: SentenceImmutables) -> SentencePointer {
        let arc = Arc::new(sentence_data);
        let sentence_pointer = SentencePointer(self.sentences.len(), arc.clone());
        self.sentences.push(SentenceAnalysis::default());
        self.sentence_immutables.push(arc);
        sentence_pointer
    }

    pub fn new_word(
        &mut self,
        word_data: WordImmutables,
        word_analysis: WordAnalysis,
    ) -> WordPointer {
        let arc = Arc::new(word_data);
        let word_pointer = WordPointer(self.word_analyses.len(), arc.clone());
        self.word_analyses.push(word_analysis);
        self.word_immutables.push(arc);
        word_pointer
    }
}

impl<P: Pointer> Index<&P> for PageAnalysis {
    type Output = P::Data;

    fn index(&self, index: &P) -> &Self::Output {
        index.data(self)
    }
}

impl<P: Pointer> IndexMut<&P> for PageAnalysis {
    fn index_mut(&mut self, index: &P) -> &mut Self::Output {
        index.data_mut(self)
    }
}

#[derive(Debug, PartialEq, Eq, thiserror::Error)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub enum AnalysisError {
    #[error("No valid revisions found")]
    NoValidRevisions,
}

/// Trait for type-safe navigation of the [`PageAnalysis`] graph.
///
/// Each node type (revision, paragraph, sentence, word) has a pointer that
/// implements this trait. A pointer combines an index into `PageAnalysis`'s
/// storage arrays with a shared reference to the node's immutable data.
///
/// Use pointers to index into `PageAnalysis`:
///
/// ```rust,ignore
/// let data: &RevisionAnalysis = &analysis[&rev_ptr];
/// ```
///
/// **Important:** A pointer is only valid for the `PageAnalysis` that created it.
/// Using a pointer from one analysis to index into a different one will return
/// unrelated data or panic if the index is out of bounds.
///
/// Implemented by [`RevisionPointer`], [`ParagraphPointer`],
/// [`SentencePointer`], and [`WordPointer`].
pub trait Pointer: Clone {
    type Data;

    fn index(&self) -> usize;
    fn value(&self) -> &str;
    fn data<'a>(&self, analysis: &'a PageAnalysis) -> &'a Self::Data;
    fn data_mut<'a>(&self, analysis: &'a mut PageAnalysis) -> &'a mut Self::Data;
}

impl Pointer for RevisionPointer {
    type Data = RevisionAnalysis;

    fn index(&self) -> usize {
        self.0
    }

    fn value(&self) -> &str {
        &self.text_lowercase
    }

    fn data<'a>(&self, analysis: &'a PageAnalysis) -> &'a Self::Data {
        &analysis.revisions[self.0]
    }

    fn data_mut<'a>(&self, analysis: &'a mut PageAnalysis) -> &'a mut Self::Data {
        &mut analysis.revisions[self.0]
    }
}

impl Pointer for ParagraphPointer {
    type Data = ParagraphAnalysis;

    fn index(&self) -> usize {
        self.0
    }

    fn value(&self) -> &str {
        &self.value
    }

    fn data<'a>(&self, analysis: &'a PageAnalysis) -> &'a Self::Data {
        &analysis.paragraphs[self.0]
    }

    fn data_mut<'a>(&self, analysis: &'a mut PageAnalysis) -> &'a mut Self::Data {
        &mut analysis.paragraphs[self.0]
    }
}

impl Pointer for SentencePointer {
    type Data = SentenceAnalysis;

    fn index(&self) -> usize {
        self.0
    }

    fn value(&self) -> &str {
        &self.value
    }

    fn data<'a>(&self, analysis: &'a PageAnalysis) -> &'a Self::Data {
        &analysis.sentences[self.0]
    }

    fn data_mut<'a>(&self, analysis: &'a mut PageAnalysis) -> &'a mut Self::Data {
        &mut analysis.sentences[self.0]
    }
}

impl Pointer for WordPointer {
    type Data = WordAnalysis;

    fn index(&self) -> usize {
        self.0
    }

    fn value(&self) -> &str {
        &self.1.value
    }

    fn data<'a>(&self, analysis: &'a PageAnalysis) -> &'a Self::Data {
        &analysis.word_analyses[self.0]
    }

    fn data_mut<'a>(&self, analysis: &'a mut PageAnalysis) -> &'a mut Self::Data {
        &mut analysis.word_analyses[self.0]
    }
}
