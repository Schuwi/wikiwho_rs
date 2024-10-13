use chrono::prelude::*;
use compact_str::CompactString;
use imara_diff::intern::Interner;
use rustc_hash::{FxHashMap, FxHashSet};
use std::{
    collections::HashMap,
    ops::{Deref, Index, IndexMut},
    sync::Arc,
};

use crate::{
    dump_parser::{Contributor, Revision, Text},
    utils::{
        self, compute_avg_word_freq, split_into_paragraphs, split_into_sentences,
        split_into_tokens, trim_in_place, ChangeTag, RevisionHash,
    },
};

#[derive(Clone)]
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
pub struct RevisionPointer(pub usize, pub Arc<RevisionImmutables>);

impl RevisionPointer {
    fn new(index: usize, revision: RevisionImmutables) -> Self {
        Self(index, Arc::new(revision))
    }
}

impl Deref for RevisionPointer {
    type Target = RevisionImmutables;

    fn deref(&self) -> &Self::Target {
        &self.1
    }
}

#[derive(Clone)]
pub struct RevisionImmutables {
    pub id: i32,
    pub length: usize, /* text length when lowercased, in bytes (for `test` compile target this is the number of unicode codepoints, to match the python implementation) */
    pub text: String,  /* lowercased text of revision */
    pub xml_revision: Revision,
}

impl RevisionImmutables {
    fn dummy() -> Self {
        Self {
            id: 0,
            length: 0,
            text: String::new(),
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
}

#[derive(Clone, Default)]
pub struct RevisionAnalysis {
    paragraphs_by_hash: FxHashMap<blake3::Hash, MaybeVec<ParagraphPointer>>, /* assume that duplicate paragraphs are not very common and optimize to avoid allocation */
    pub paragraphs_ordered: Vec<ParagraphPointer>,

    pub original_adds: usize, /* number of tokens added in this revision */
}

impl RevisionImmutables {
    pub fn from_revision(revision: &Revision) -> Self {
        Self {
            id: revision.id,
            // #[cfg(not(any(test, feature = "match-reference-impl")))]
            // // for spam detection it should be enough to use the length of the text in bytes
            // length: revision.text.len(),
            // #[cfg(any(test, feature = "match-reference-impl"))]
            // python's `len` function returns the number of unicode codepoints for a string,
            // so when testing against the python implementation we need to match that behavior to get identical results
            length: revision.text.as_str().chars().count(),
            text: match revision.text {
                Text::Normal(ref t) => utils::to_lowercase(t),
                Text::Deleted => String::new(),
            },
            xml_revision: revision.clone(),
        }
    }
}

// index is unique within a page
#[derive(Clone)]
pub struct ParagraphPointer(pub usize, pub Arc<ParagraphImmutables>);

impl ParagraphPointer {
    fn new(index: usize, paragraph: ParagraphImmutables) -> Self {
        Self(index, Arc::new(paragraph))
    }
}

impl Deref for ParagraphPointer {
    type Target = ParagraphImmutables;

    fn deref(&self) -> &Self::Target {
        &self.1
    }
}

#[derive(Clone)]
pub struct ParagraphImmutables {
    hash_value: blake3::Hash,
    pub value: String,
}

#[derive(Clone, Default)]
pub struct ParagraphAnalysis {
    sentences_by_hash: FxHashMap<blake3::Hash, MaybeVec<SentencePointer>>,
    pub sentences_ordered: Vec<SentencePointer>,

    /// whether this paragraph was found in the current revision
    pub matched_in_current: bool,
}

impl ParagraphImmutables {
    pub fn new(value: String) -> Self {
        let hash_value = blake3::hash(value.as_bytes());
        Self { hash_value, value }
    }
}

// index is unique within a page
#[derive(Clone)]
pub struct SentencePointer(pub usize, pub Arc<SentenceImmutables>);

impl SentencePointer {
    fn new(index: usize, sentence: SentenceImmutables) -> Self {
        Self(index, Arc::new(sentence))
    }
}

impl Deref for SentencePointer {
    type Target = SentenceImmutables;

    fn deref(&self) -> &Self::Target {
        &self.1
    }
}

#[derive(Clone)]
pub struct SentenceImmutables {
    hash_value: blake3::Hash,
    pub value: String,
}

#[derive(Clone, Default)]
pub struct SentenceAnalysis {
    pub words_ordered: Vec<WordPointer>,

    /// whether this sentence was found in the current revision
    pub matched_in_current: bool,
}

impl SentenceImmutables {
    pub fn new(value: String) -> Self {
        let hash_value = blake3::hash(value.as_bytes());
        Self { hash_value, value }
    }
}

// index is unique within a page
#[derive(Clone)]
pub struct WordPointer(pub usize, pub Arc<WordImmutables>);

impl WordPointer {
    fn new(index: usize, word: WordImmutables) -> Self {
        Self(index, Arc::new(word))
    }

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

#[derive(Clone)]
pub struct WordImmutables {
    pub value: CompactString,
}

#[derive(Clone)]
pub struct WordAnalysis {
    pub origin_rev_id: i32,
    pub latest_rev_id: i32,
    /// whether this word was found in the current revision
    pub matched_in_current: bool,

    // words may be re-added after being removed
    pub inbound: Vec<i32>,
    pub outbound: Vec<i32>, // the revision ids where this word was removed (i.e. not present in the revision but present in the previous revision)
}

impl WordImmutables {
    pub fn new(value: CompactString) -> Self {
        Self { value }
    }
}

impl WordAnalysis {
    pub fn new(_pointer: WordPointer, origin_rev_id: i32) -> Self {
        Self {
            origin_rev_id,
            latest_rev_id: origin_rev_id,
            matched_in_current: false,
            inbound: Vec::new(),
            outbound: Vec::new(),
        }
    }

    fn maybe_push_inbound(
        &mut self,
        vandalism: bool,
        revision_id_curr: i32,
        revision_id_prev: Option<i32>,
        push: bool,
    ) {
        if !vandalism && self.matched_in_current && self.outbound.last() != Some(&revision_id_curr)
        {
            if push && Some(self.latest_rev_id) != revision_id_prev {
                self.inbound.push(revision_id_curr);
            }
            self.latest_rev_id = revision_id_curr;
        }
    }

    fn maybe_push_outbound(&mut self, revision_id_curr: i32) {
        if !self.matched_in_current {
            self.outbound.push(revision_id_curr);
        }
    }
}

#[derive(Default)]
struct AnalysisInternals {
    paragraphs_ht: FxHashMap<blake3::Hash, Vec<ParagraphPointer>>, // Hash table of paragraphs of all revisions
    sentences_ht: FxHashMap<blake3::Hash, Vec<SentencePointer>>, // Hash table of sentences of all revisions
    spam_hashes: FxHashSet<RevisionHash>, // Hashes of spam revisions; RevisionHash can be a SHA1 hash or a BLAKE3 hash but we expect all hashes in this revision to be of the same type

    revision_prev: Option<RevisionPointer>,
    // text_curr: String, /* pass text_curr as parameter instead */
    // temp: Vec<String>, /* replaced by disambiguate_* in analyse_page */
    scratch_buffers: (String, String),
}

pub struct Analysis {
    // single array where the structural and analytical information of all the revisions/paragraphs/sentences/words in this page is stored
    // the goal is to work with Rust's memory model and avoid falling back to Arc<RefCell<...>> everywhere
    pub revisions: Vec<RevisionAnalysis>,
    pub paragraphs: Vec<ParagraphAnalysis>,
    pub sentences: Vec<SentenceAnalysis>,
    pub words: Vec<WordAnalysis>, // Ordered, unique list of tokens in the page

    /// Collection of revision IDs that were detected as spam.
    pub spam_ids: Vec<i32>,
    /// Map of revision ID to RevisionData.
    ///
    /// Does not contain revisions that were detected as spam.
    pub revisions_by_id: HashMap<i32, RevisionPointer>,
    /// List of revision IDs in order from oldest to newest.
    ///
    /// Does not contain revisions that were detected as spam.
    pub ordered_revisions: Vec<i32>,

    /// The current revision being analysed.
    ///
    /// After analysis finished this will be the latest revision that was not marked as spam.
    pub revision_curr: RevisionPointer,

    internals: AnalysisInternals,
}

impl<P: Pointer> Index<&P> for Analysis {
    type Output = P::Data;

    fn index(&self, index: &P) -> &Self::Output {
        index.data(self)
    }
}

impl<P: Pointer> IndexMut<&P> for Analysis {
    fn index_mut(&mut self, index: &P) -> &mut Self::Output {
        index.data_mut(self)
    }
}

// Spam detection variables.
// use f64 instead of f32 to replicate the behavior of the Python script
const CHANGE_PERCENTAGE: f64 = -0.40;
const PREVIOUS_LENGTH: usize = 1000;
const CURR_LENGTH: usize = 1000;
const UNMATCHED_PARAGRAPH: f64 = 0.0;
const TOKEN_DENSITY_LIMIT: f64 = 20.0;

#[derive(Debug, PartialEq, Eq)]
pub enum AnalysisError {
    NoValidRevisions,
}

pub trait Pointer: Clone {
    type Data;

    fn index(&self) -> usize;
    fn value(&self) -> &str;
    fn data<'a>(&self, analysis: &'a Analysis) -> &'a Self::Data;
    fn data_mut<'a>(&self, analysis: &'a mut Analysis) -> &'a mut Self::Data;
}

impl Pointer for RevisionPointer {
    type Data = RevisionAnalysis;

    fn index(&self) -> usize {
        self.0
    }

    fn value(&self) -> &str {
        &self.text
    }

    fn data<'a>(&self, analysis: &'a Analysis) -> &'a Self::Data {
        &analysis.revisions[self.0]
    }

    fn data_mut<'a>(&self, analysis: &'a mut Analysis) -> &'a mut Self::Data {
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

    fn data<'a>(&self, analysis: &'a Analysis) -> &'a Self::Data {
        &analysis.paragraphs[self.0]
    }

    fn data_mut<'a>(&self, analysis: &'a mut Analysis) -> &'a mut Self::Data {
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

    fn data<'a>(&self, analysis: &'a Analysis) -> &'a Self::Data {
        &analysis.sentences[self.0]
    }

    fn data_mut<'a>(&self, analysis: &'a mut Analysis) -> &'a mut Self::Data {
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

    fn data<'a>(&self, analysis: &'a Analysis) -> &'a Self::Data {
        &analysis.words[self.0]
    }

    fn data_mut<'a>(&self, analysis: &'a mut Analysis) -> &'a mut Self::Data {
        &mut analysis.words[self.0]
    }
}

// since the handling of paragraphs and sentences is almost identical, we generalize
trait ParasentPointer: Sized + Pointer {
    type ParentPointer: Pointer;
    const IS_SENTENCE: bool;

    fn allocate_new_in_parent(
        analysis: &mut Analysis,
        parent: &Self::ParentPointer,
        text: String,
    ) -> Self;

    fn iterate_words(analysis: &mut Analysis, parasents: &[Self], f: impl FnMut(&mut WordAnalysis));
    fn all_parasents_in_parents(
        analysis: &mut Analysis,
        prevs: &[Self::ParentPointer],
    ) -> Vec<Self>;
    fn find_in_parents(
        analysis: &mut Analysis,
        prevs: &[Self::ParentPointer],
        hash: &blake3::Hash,
    ) -> Vec<Self>;
    fn store_in_parent(&self, analysis: &mut Analysis, curr: &Self::ParentPointer);
    fn find_in_any_previous_revision(analysis: &mut Analysis, hash: &blake3::Hash) -> Vec<Self>;

    fn split_into_parasents(
        parasent_text: &str,
        scratch_buffers: (&mut String, &mut String),
    ) -> Vec<String>;

    fn mark_all_children_matched(&self, analysis: &mut Analysis);

    fn matched_in_current(&self, analysis: &mut Analysis) -> bool;
    fn set_matched_in_current(&self, analysis: &mut Analysis, value: bool);
}

impl ParasentPointer for ParagraphPointer {
    type ParentPointer = RevisionPointer;
    const IS_SENTENCE: bool = false;

    fn allocate_new_in_parent(
        analysis: &mut Analysis,
        parent: &RevisionPointer,
        text: String,
    ) -> Self {
        let paragraph_data = ParagraphImmutables::new(text);
        let paragraph_pointer = ParagraphPointer::new(analysis.paragraphs.len(), paragraph_data);
        analysis.paragraphs.push(ParagraphAnalysis::default());

        let revision_curr = &mut analysis.revisions[parent.0];
        revision_curr
            .paragraphs_by_hash
            .entry(paragraph_pointer.hash_value)
            .and_modify(|v| v.push(paragraph_pointer.clone()))
            .or_insert_with(|| MaybeVec::new_single(paragraph_pointer.clone()));
        revision_curr
            .paragraphs_ordered
            .push(paragraph_pointer.clone());
        paragraph_pointer
    }

    fn iterate_words(
        analysis: &mut Analysis,
        paragraphs: &[Self],
        f: impl FnMut(&mut WordAnalysis),
    ) {
        analysis.iterate_words_in_paragraphs(paragraphs, f);
    }

    fn all_parasents_in_parents(analysis: &mut Analysis, prevs: &[RevisionPointer]) -> Vec<Self> {
        let mut result = Vec::new();
        for revision_prev in prevs {
            result.extend_from_slice(&analysis.revisions[revision_prev.0].paragraphs_ordered);
        }
        result
    }

    fn split_into_parasents(
        revision_text: &str,
        scratch_buffers: (&mut String, &mut String),
    ) -> Vec<String> {
        // Split the text of the current revision into paragraphs.
        let paragraphs = split_into_paragraphs(revision_text, scratch_buffers);
        paragraphs
            .into_iter()
            .map(trim_in_place)
            .filter(|s| !s.is_empty()) /* don't track empty paragraphs */
            .collect()
    }

    fn find_in_parents(
        analysis: &mut Analysis,
        prevs: &[RevisionPointer],
        hash: &blake3::Hash,
    ) -> Vec<Self> {
        let mut result = Vec::new();
        for revision_prev in prevs {
            if let Some(paragraphs) = analysis.revisions[revision_prev.0]
                .paragraphs_by_hash
                .get(hash)
            {
                result.extend_from_slice(paragraphs.as_slice());
            }
        }
        result
    }

    fn store_in_parent(&self, analysis: &mut Analysis, curr: &Self::ParentPointer) {
        let revision_curr = &mut analysis.revisions[curr.0];
        revision_curr
            .paragraphs_by_hash
            .entry(self.hash_value)
            .and_modify(|v| v.push(self.clone()))
            .or_insert_with(|| MaybeVec::new_single(self.clone()));
        revision_curr.paragraphs_ordered.push(self.clone());
    }

    fn find_in_any_previous_revision(analysis: &mut Analysis, hash: &blake3::Hash) -> Vec<Self> {
        analysis
            .internals
            .paragraphs_ht
            .get(hash)
            .cloned()
            .unwrap_or_default()
    }

    fn mark_all_children_matched(&self, analysis: &mut Analysis) {
        for sentence in &analysis.paragraphs[self.0].sentences_ordered {
            analysis.sentences[sentence.0].matched_in_current = true;
            for word in &analysis.sentences[sentence.0].words_ordered {
                analysis.words[word.0].matched_in_current = true;
            }
        }
    }

    fn matched_in_current(&self, analysis: &mut Analysis) -> bool {
        analysis.paragraphs[self.0].matched_in_current
    }

    fn set_matched_in_current(&self, analysis: &mut Analysis, value: bool) {
        analysis.paragraphs[self.0].matched_in_current = value;
    }
}

impl ParasentPointer for SentencePointer {
    type ParentPointer = ParagraphPointer;
    const IS_SENTENCE: bool = true;

    fn allocate_new_in_parent(
        analysis: &mut Analysis,
        parent: &ParagraphPointer,
        text: String,
    ) -> Self {
        let sentence_data = SentenceImmutables::new(text);
        let sentence_pointer = SentencePointer::new(analysis.sentences.len(), sentence_data);
        analysis.sentences.push(SentenceAnalysis::default());

        let paragraph_curr = &mut analysis.paragraphs[parent.0];
        paragraph_curr
            .sentences_by_hash
            .entry(sentence_pointer.hash_value)
            .and_modify(|v| v.push(sentence_pointer.clone()))
            .or_insert_with(|| MaybeVec::new_single(sentence_pointer.clone()));
        paragraph_curr
            .sentences_ordered
            .push(sentence_pointer.clone());
        sentence_pointer
    }

    fn iterate_words(
        analysis: &mut Analysis,
        sentences: &[Self],
        f: impl FnMut(&mut WordAnalysis),
    ) {
        analysis.iterate_words_in_sentences(sentences, f);
    }

    fn all_parasents_in_parents(analysis: &mut Analysis, prevs: &[ParagraphPointer]) -> Vec<Self> {
        let mut result = Vec::new();
        for paragraph_prev in prevs {
            result.extend_from_slice(&analysis.paragraphs[paragraph_prev.0].sentences_ordered);
        }
        result
    }

    fn split_into_parasents(
        paragraph_text: &str,
        scratch_buffers: (&mut String, &mut String),
    ) -> Vec<String> {
        // Split the current paragraph into sentences.
        let sentences = split_into_sentences(paragraph_text, scratch_buffers);
        sentences
            .into_iter()
            .map(trim_in_place)
            .filter(|s| !s.is_empty()) /* don't track empty sentences */
            .map(|s| split_into_tokens(&s).join(" ")) /* here whitespaces in the sentence are cleaned */
            .collect()
    }

    fn find_in_parents(
        analysis: &mut Analysis,
        unmatched_paragraphs_prev: &[ParagraphPointer],
        hash: &blake3::Hash,
    ) -> Vec<Self> {
        let mut result = Vec::new();
        for paragraph_prev in unmatched_paragraphs_prev {
            if let Some(sentences) = analysis.paragraphs[paragraph_prev.0]
                .sentences_by_hash
                .get(hash)
            {
                result.extend_from_slice(sentences.as_slice());
            }
        }
        result
    }

    fn store_in_parent(&self, analysis: &mut Analysis, curr: &Self::ParentPointer) {
        let paragraph_curr = &mut analysis.paragraphs[curr.0];
        paragraph_curr
            .sentences_by_hash
            .entry(self.hash_value)
            .and_modify(|v| v.push(self.clone()))
            .or_insert_with(|| MaybeVec::new_single(self.clone()));
        paragraph_curr.sentences_ordered.push(self.clone());
    }

    fn find_in_any_previous_revision(analysis: &mut Analysis, hash: &blake3::Hash) -> Vec<Self> {
        analysis
            .internals
            .sentences_ht
            .get(hash)
            .cloned()
            .unwrap_or_default()
    }

    fn mark_all_children_matched(&self, analysis: &mut Analysis) {
        for word in &analysis.sentences[self.0].words_ordered {
            analysis.words[word.0].matched_in_current = true;
        }
    }

    fn matched_in_current(&self, analysis: &mut Analysis) -> bool {
        analysis.sentences[self.0].matched_in_current
    }

    fn set_matched_in_current(&self, analysis: &mut Analysis, value: bool) {
        analysis.sentences[self.0].matched_in_current = value;
    }
}

impl Analysis {
    pub fn analyse_page(xml_revisions: &[Revision]) -> Result<Self, AnalysisError> {
        let mut analysis = Self {
            revisions: Vec::new(),
            paragraphs: Vec::new(),
            sentences: Vec::new(),
            words: Vec::new(),

            spam_ids: Vec::new(),
            revisions_by_id: HashMap::new(),
            ordered_revisions: Vec::new(),

            revision_curr: RevisionPointer::new(0, RevisionImmutables::dummy()), /* will be overwritten before being read */

            internals: AnalysisInternals {
                paragraphs_ht: FxHashMap::default(),
                sentences_ht: FxHashMap::default(),
                spam_hashes: FxHashSet::default(),
                revision_prev: None,

                scratch_buffers: (String::new(), String::new()),
            },
        };

        let mut at_least_one = false;

        // Iterate over revisions of the article.
        // Analysis begins at the oldest revision and progresses to the newest.
        for xml_revision in xml_revisions {
            // Extract text of the revision
            let text = match xml_revision.text {
                Text::Normal(ref t) => t,
                Text::Deleted => {
                    // Skip revisions with deleted text
                    continue;
                }
            };

            // Use pre-calculated SHA1 hash if available, otherwise calculate BLAKE3 hash
            let rev_hash = match xml_revision.sha1 {
                Some(sha1_hash) => RevisionHash::Sha1(sha1_hash),
                None => RevisionHash::Blake3(blake3::hash(text.as_bytes())),
            };

            let revision_data = RevisionImmutables::from_revision(xml_revision);
            let mut vandalism = false;

            if analysis.internals.spam_hashes.contains(&rev_hash) {
                // The content of this revision has already been marked as spam
                vandalism = true;
            }

            // Spam detection: Deletion
            if !(vandalism || xml_revision.comment.is_some() && xml_revision.minor) {
                let revision_prev = &analysis.revision_curr; /* !! since we have not yet updated revision_curr, this is the previous revision */
                let change_percentage = (revision_data.length as f64 - revision_prev.length as f64)
                    / revision_prev.length as f64;

                if revision_prev.length > PREVIOUS_LENGTH
                    && revision_data.length < CURR_LENGTH
                    && change_percentage <= CHANGE_PERCENTAGE
                {
                    // Vandalism detected due to significant deletion
                    vandalism = true;
                }
            }

            if vandalism {
                // Skip this revision, treat it as spam
                analysis.spam_ids.push(revision_data.id);
                analysis.internals.spam_hashes.insert(rev_hash);
                continue;
            }

            // Allocate a new revision and create a pointer to it.
            let mut revision_pointer =
                RevisionPointer::new(analysis.revisions.len(), revision_data);
            analysis.revisions.push(RevisionAnalysis::default());

            // Update the information about the previous revision.
            std::mem::swap(&mut analysis.revision_curr, &mut revision_pointer);
            if at_least_one {
                analysis.internals.revision_prev = Some(revision_pointer);
            } /* if !at_least_one we do not yet have a valid revision (revision_pointer contains a dummy value) to refer to as previous */

            // Perform the actual word (aka. token) matching
            vandalism = analysis.determine_authorship();

            if vandalism {
                // Skip this revision due to vandalism
                if at_least_one {
                    // Revert the state of `revision_curr` to the beginning of the loop iteration
                    analysis.revision_curr =
                        analysis.internals.revision_prev.take().expect(
                            "should not have been deleted in the call to determine_authorship",
                        );
                } /* while !at_least_one we expect revision_prev to be None */

                // Mark the revision as spam
                analysis.spam_ids.push(xml_revision.id);
                analysis.internals.spam_hashes.insert(rev_hash);
            } else {
                // Store the current revision in the result
                analysis.ordered_revisions.push(analysis.revision_curr.id);
                analysis
                    .revisions_by_id
                    .insert(analysis.revision_curr.id, analysis.revision_curr.clone());

                // and note that we have processed at least one valid revision
                at_least_one = true;
            }
        }

        if !at_least_one {
            Err(AnalysisError::NoValidRevisions)
        } else {
            Ok(analysis)
        }
    }

    // fn iterate_words(&mut self, words: &[WordPointer], mut f: impl FnMut(&mut WordAnalysis)) {
    //     for word in words {
    //         f(&mut self.words[word.0]);
    //     }
    // }

    fn iterate_words_in_sentences(
        &mut self,
        sentences: &[SentencePointer],
        mut f: impl FnMut(&mut WordAnalysis),
    ) {
        for sentence in sentences {
            for word in &self.sentences[sentence.0].words_ordered {
                f(&mut self.words[word.0]);
            }
        }
    }

    fn iterate_words_in_paragraphs(
        &mut self,
        paragraphs: &[ParagraphPointer],
        mut f: impl FnMut(&mut WordAnalysis),
    ) {
        for paragraph in paragraphs {
            for sentence in &self.paragraphs[paragraph.0].sentences_ordered {
                for word in &self.sentences[sentence.0].words_ordered {
                    f(&mut self.words[word.0]);
                }
            }
        }
    }

    // fn iterate_words_in_revisions(
    //     &mut self,
    //     revisions: &[RevisionPointer],
    //     mut f: impl FnMut(&mut WordAnalysis),
    // ) {
    //     for revision in revisions {
    //         for paragraph in &self.revisions[revision.0].paragraphs_ordered {
    //             for sentence in &self.paragraphs[paragraph.0].sentences_ordered {
    //                 for word in &self.sentences[sentence.0].words_ordered {
    //                     f(&mut self.words[word.0]);
    //                 }
    //             }
    //         }
    //     }
    // }

    fn determine_authorship(&mut self) -> bool {
        /*
        unmatched_paragraphs_{prev, curr}
        unmatched_sentences_{prev, curr}

        matched_{paragraphs, words, sentences}_prev
         */
        let revision_id_curr = self.revision_curr.id; /* short-hand */
        let revision_id_prev = self.internals.revision_prev.as_ref().map(|r| r.id); /* short-hand */

        let mut unmatched_sentences_curr = Vec::new();
        let mut unmatched_sentences_prev = Vec::new();

        let mut matched_sentences_prev = Vec::new();
        let mut matched_words_prev = Vec::new();

        let mut possible_vandalism = false;
        let mut vandalism = false;

        // Analysis of the paragraphs in the current revision
        let (unmatched_paragraphs_curr, unmatched_paragraphs_prev, matched_paragraphs_prev, _) =
            self.analyse_parasents_in_revgraph(
                &[self.revision_curr.clone()],
                self.internals.revision_prev.as_ref().cloned().as_slice(),
            );

        if !unmatched_paragraphs_curr.is_empty() {
            // there are some paragraphs for us to match
            let result = self.analyse_parasents_in_revgraph(
                &unmatched_paragraphs_curr,
                &unmatched_paragraphs_prev,
            );

            unmatched_sentences_curr = result.0;
            unmatched_sentences_prev = result.1;
            matched_sentences_prev = result.2;

            // this will always set possible_vandalism to true (because UNMATCHED_PARAGRAPH is 0.0)
            if unmatched_paragraphs_curr.len() as f64
                / self[&self.revision_curr].paragraphs_ordered.len() as f64
                > UNMATCHED_PARAGRAPH
            {
                // will be used to detect copy-paste vandalism - token density
                possible_vandalism = true;
            }

            if !unmatched_sentences_curr.is_empty() {
                // there are some **sentences** for us to match
                let result = self.analyse_words_in_sentences(
                    &unmatched_sentences_curr,
                    &unmatched_sentences_prev,
                    possible_vandalism,
                );

                matched_words_prev = result.0;
                vandalism = result.1;
            }
        }

        if !vandalism {
            // tag all words that are deleted in the current revision (i.e. present in the previous revision but not in the current revision)
            self.iterate_words_in_sentences(&unmatched_sentences_prev, |word| {
                word.maybe_push_outbound(revision_id_curr)
            });

            // ???
            if unmatched_sentences_prev.is_empty() {
                self.iterate_words_in_paragraphs(&unmatched_paragraphs_prev, |word| {
                    word.maybe_push_outbound(revision_id_curr)
                });
            }

            // Add the new paragraphs to the hash table
            for paragraph in unmatched_paragraphs_curr {
                let hash = paragraph.hash_value;
                self.internals
                    .paragraphs_ht
                    .entry(hash)
                    .or_default()
                    .push(paragraph.clone());
            }

            // Add the new sentences to the hash table
            for sentence in unmatched_sentences_curr {
                let hash = sentence.hash_value;
                self.internals
                    .sentences_ht
                    .entry(hash)
                    .or_default()
                    .push(sentence.clone());
            }
        }

        // Reset the matches that we modified in old revisions
        let handle_word = |word: &mut WordAnalysis, push_inbound: bool| {
            // first update inbound and last used info of matched words of all previous revisions
            word.maybe_push_inbound(vandalism, revision_id_curr, revision_id_prev, push_inbound);
            // then reset the matched status
            word.matched_in_current = false;
        };

        for matched_paragraph in &matched_paragraphs_prev {
            matched_paragraph.set_matched_in_current(self, false);
            for matched_sentence in &self.paragraphs[matched_paragraph.0].sentences_ordered {
                self.sentences[matched_sentence.0].matched_in_current = false;

                for matched_word in &self.sentences[matched_sentence.0].words_ordered {
                    handle_word(&mut self.words[matched_word.0], true);
                }
            }
        }
        for matched_sentence in &matched_sentences_prev {
            matched_sentence.set_matched_in_current(self, false);

            for matched_word in &self.sentences[matched_sentence.0].words_ordered {
                handle_word(&mut self.words[matched_word.0], true);
            }
        }
        for matched_word in &matched_words_prev {
            // there is no inbound chance because we only diff with words of previous revision -> push_inbound = false
            handle_word(&mut self.words[matched_word.0], false);
        }

        vandalism
    }

    fn find_matching_parasent<P: ParasentPointer>(
        /* T is ParagraphPointer or SentencePointer */
        &mut self,
        prev_parasents: &[P],
        matched_parasents_prev: &mut Vec<P>,
    ) -> Option<P> {
        for parasent_prev_pointer in prev_parasents {
            if parasent_prev_pointer.matched_in_current(self) {
                // skip paragraphs that have already been matched
                continue;
            }

            let mut matched_one = false;
            let mut matched_all = true;

            P::iterate_words(self, &[parasent_prev_pointer.clone()], |word| {
                if word.matched_in_current {
                    matched_one = true;
                } else {
                    matched_all = false;
                }
            });

            if !matched_one {
                // no words in this paragraph are matched yet
                parasent_prev_pointer.set_matched_in_current(self, true);
                matched_parasents_prev.push(parasent_prev_pointer.clone());

                // no need to check other paragraphs
                return Some(parasent_prev_pointer.clone());
            } else if matched_all {
                // all words in this paragraph are matched
                parasent_prev_pointer.set_matched_in_current(self, true);
                matched_parasents_prev.push(parasent_prev_pointer.clone());
            }
        }
        None
    }

    fn analyse_parasents_in_revgraph<P: ParasentPointer>(
        /* revgraph = revision + paragraph */
        &mut self,
        unmatched_revgraphs_curr: &[P::ParentPointer], /* for paragraphs_in_revision this is just &[self.revision_curr] */
        unmatched_revgraphs_prev: &[P::ParentPointer], /* for paragraphs_in_revision this is just &[self.revision_prev] or &[] */
    ) -> (Vec<P>, Vec<P>, Vec<P>, usize) {
        let mut unmatched_parasents_curr = Vec::new();
        let mut unmatched_parasents_prev = Vec::new();
        let mut matched_parasents_prev = Vec::new();
        let mut total_parasents = 0;

        // Iterate over the unmatched paragraphs/sentences in the current revision/paragraph
        for parasent_curr_pointer in unmatched_revgraphs_curr {
            // split the text
            let parasents = P::split_into_parasents(
                parasent_curr_pointer.value(),
                (
                    &mut self.internals.scratch_buffers.0,
                    &mut self.internals.scratch_buffers.1,
                ),
            );

            // iterate over the paragraphs/sentences in the current revision/paragraph
            for parasent_text in parasents {
                let hash_curr = blake3::hash(parasent_text.as_bytes());
                let mut matched_curr; /* whether we found a match for this parasent in any previous revgraph */

                total_parasents += 1;

                // Check if this parasent exists unmatched in the previous revision
                let prev_parasents = P::find_in_parents(self, unmatched_revgraphs_prev, &hash_curr);
                matched_curr = self
                    .find_matching_parasent(prev_parasents.as_slice(), &mut matched_parasents_prev);

                if matched_curr.is_none() {
                    // this parasent was not found in the previous revision
                    // check if it is in an older revision
                    let prev_paragraphs = P::find_in_any_previous_revision(self, &hash_curr);
                    matched_curr = self.find_matching_parasent(
                        prev_paragraphs.as_slice(),
                        &mut matched_parasents_prev,
                    );
                }

                if let Some(parasent_prev_pointer) = matched_curr {
                    // this parasent was found in a previous revision

                    // Mark all sentences and words in this paragraph/sentence as matched
                    parasent_prev_pointer.mark_all_children_matched(self);

                    // Add paragraph/sentence to the current revision/paragraph
                    parasent_prev_pointer.store_in_parent(self, parasent_curr_pointer);
                } else {
                    // this paragraph/sentence was not found in any previous revision, so it is new
                    // add to the list of unmatched paragraphs/sentences for future matching

                    // Allocate a new paragraph/sentence and create a pointer to it.
                    let paragraph_pointer =
                        P::allocate_new_in_parent(self, parasent_curr_pointer, parasent_text);
                    unmatched_parasents_curr.push(paragraph_pointer);
                }
            }
        }

        // Identify unmatched paragraphs/sentences in the previous revision/paragraph
        for parasent_prev_pointer in P::all_parasents_in_parents(self, unmatched_revgraphs_prev) {
            if !parasent_prev_pointer.matched_in_current(self) {
                unmatched_parasents_prev.push(parasent_prev_pointer.clone());

                if P::IS_SENTENCE {
                    // to reset 'matched words in analyse_words_in_sentences' of unmatched paragraphs and sentences
                    parasent_prev_pointer.set_matched_in_current(self, true);
                    matched_parasents_prev.push(parasent_prev_pointer);
                }
            }
        }

        (
            unmatched_parasents_curr,
            unmatched_parasents_prev,
            matched_parasents_prev,
            total_parasents,
        )
    }

    ///
    /// # Returns
    ///
    /// (matched_words_prev, possible_vandalism)
    fn analyse_words_in_sentences(
        &mut self,
        unmatched_sentences_curr: &[SentencePointer],
        unmatched_sentences_prev: &[SentencePointer],
        possible_vandalism: bool,
    ) -> (Vec<WordPointer>, bool) {
        // estimate the number of unique unmatched words in all unmatched sentences (prev and curr)
        let upper_bound_tokens = unmatched_sentences_curr
            .iter()
            .chain(unmatched_sentences_prev.iter())
            .map(|sentence_pointer| self.sentences[sentence_pointer.0].words_ordered.len())
            .sum::<usize>();

        let mut interner = Interner::new(upper_bound_tokens);
        let mut matched_words_prev = Vec::new();
        let mut unmatched_words_prev = Vec::new();

        // Split sentences into words.
        let mut text_prev = Vec::new();
        for sentence_prev_pointer in unmatched_sentences_prev {
            let sentence_prev = &self.sentences[sentence_prev_pointer.0];
            for word_prev_pointer in &sentence_prev.words_ordered {
                if !self.words[word_prev_pointer.0].matched_in_current {
                    let interned = interner.intern(word_prev_pointer.value().to_string());
                    text_prev.push(interned);
                    unmatched_words_prev.push((interned, word_prev_pointer.clone()));
                }
            }
        }

        let mut unmatched_sentence_curr_splitted = Vec::new();
        let mut text_curr = Vec::new();
        for sentence_curr_pointer in unmatched_sentences_curr {
            // split_into_tokens is already done in analyse_sentences_in_paragraphs
            let words: Vec<_> = sentence_curr_pointer
                .value()
                //.split_whitespace() // DU SCHLINGEL!!! :D
                .split(' ')
                .map(|s| interner.intern(s.to_string()))
                .collect();
            text_curr.extend_from_slice(words.as_slice());
            unmatched_sentence_curr_splitted.push(words); /* index corresponds to index in unmatched_words_prev */
        }

        if text_curr.is_empty() {
            // Edit consists of removing sentences, not adding new content.
            return (matched_words_prev, false);
        }

        // spam detection. Check if the token density is too high.
        if possible_vandalism {
            let token_density = compute_avg_word_freq(&text_curr, &mut interner);
            if token_density > TOKEN_DENSITY_LIMIT {
                return (matched_words_prev, true);
            }
        }

        fn allocate_new_word(
            analysis: &mut Analysis,
            word: &str,
            sentence_pointer: &SentencePointer,
        ) {
            let word_data = WordImmutables::new(word.into());
            let word_pointer = WordPointer::new(analysis.words.len(), word_data);
            analysis.words.push(WordAnalysis::new(
                word_pointer.clone(),
                analysis.revision_curr.id,
            ));
            analysis.sentences[sentence_pointer.0]
                .words_ordered
                .push(word_pointer);
            analysis.revisions[analysis.revision_curr.0].original_adds += 1;
        }

        // Edit consists of adding new content, not changing/removing content
        if text_prev.is_empty() {
            for (i, sentence_curr_pointer) in unmatched_sentences_curr.iter().enumerate() {
                for word_interned in unmatched_sentence_curr_splitted[i].iter() {
                    allocate_new_word(self, &interner[*word_interned], sentence_curr_pointer);
                }
            }
            return (matched_words_prev, false);
        }

        // do the diffing!
        let mut diff: Vec<_>;
        if cfg!(feature = "python-diff") {
            diff = utils::python_diff(&text_prev, &text_curr, &mut interner);
        } else {
            diff = utils::imara_diff(&text_prev, &text_curr, interner.num_tokens());
        }

        for (i, sentence_curr) in unmatched_sentences_curr.iter().enumerate() {
            for word_interned in unmatched_sentence_curr_splitted[i].iter() {
                let mut curr_matched = false;
                for change in diff.iter_mut().filter(|c| c.is_some()) {
                    let (change_tag, change_value) = change.as_ref().unwrap();

                    if change_value == word_interned {
                        match change_tag {
                            ChangeTag::Equal => {
                                // match
                                if let Some((_, word_prev)) =
                                    unmatched_words_prev.iter().find(|(w_interned, w_pointer)| {
                                        w_interned == word_interned
                                            && !self.words[w_pointer.0].matched_in_current
                                    })
                                {
                                    curr_matched = true;

                                    self[word_prev].matched_in_current = true;
                                    self[sentence_curr].words_ordered.push(word_prev.clone());

                                    matched_words_prev.push(word_prev.clone());
                                    *change = None;
                                }
                            }
                            ChangeTag::Delete => {
                                // word was deleted
                                if let Some((_, word_prev)) =
                                    unmatched_words_prev.iter().find(|(w_interned, w_pointer)| {
                                        w_interned == word_interned
                                            && !self.words[w_pointer.0].matched_in_current
                                    })
                                {
                                    self[word_prev].matched_in_current = true;

                                    let revision_curr_id = self.revision_curr.id; /* need to get id first, otherwise borrow-checker complains */
                                    self[word_prev].outbound.push(revision_curr_id);

                                    matched_words_prev.push(word_prev.clone());
                                    *change = None;
                                }
                            }
                            ChangeTag::Insert => {
                                // a new added word
                                curr_matched = true;

                                allocate_new_word(self, &interner[*word_interned], sentence_curr);

                                *change = None;
                            }
                        }
                        if curr_matched {
                            break;
                        }
                    }
                }

                if !curr_matched {
                    // word was not found in the diff
                    // apparently we are adding it as a new one
                    allocate_new_word(self, &interner[*word_interned], sentence_curr);
                }
            }
        }

        (matched_words_prev, false)
    }
}
