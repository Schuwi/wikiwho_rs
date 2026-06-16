// SPDX-License-Identifier: MIT AND MPL-2.0
//! Token-level authorship analysis — the WikiWho algorithm.
//!
//! This module turns an ordered sequence of [`Revision`]s
//! into a [`PageAnalysis`]: a graph of paragraphs, sentences and tokens (≈ words)
//! that records, for every token still present in a revision, the revision in
//! which it was first introduced (its *origin*) together with its add/delete
//! history across the page's lifetime.
//!
//! # Entry points
//!
//! - [`PageAnalysis::analyse_page`] — analyse a page with default options.
//! - [`PageAnalysis::analyse_page_with_options`] — same, but select algorithm
//!   behavior via [`PageAnalysisOptions`] (e.g. the `python-diff` diff algorithm or
//!   optimized non-ASCII lowercasing).
//!
//! Consume the result with
//! [`iterate_revision_tokens`](crate::utils::iterate_revision_tokens), which walks
//! the tokens of a revision in reading order.
//!
//! Revisions must be supplied in chronological order (oldest first): the algorithm
//! relies on the order of the input sequence, not on the `timestamp` field of each
//! [`Revision`].
mod types;
use std::{
    borrow::{Borrow, Cow},
    collections::HashMap,
};

pub use types::*;

#[cfg(feature = "serde")]
mod serde_impl;

use imara_diff::Interner;
use rustc_hash::{FxHashMap, FxHashSet};

use crate::{
    dump_parser::{Revision, Text},
    utils::{
        self, compute_avg_word_freq, split_into_paragraphs, split_into_sentences,
        split_into_tokens, trim_in_place, ChangeTag, RevisionHash,
    },
};

impl WordAnalysis {
    fn maybe_push_inbound(
        &mut self,
        vandalism: bool,
        revision_curr: &RevisionPointer,
        revision_prev: Option<&RevisionPointer>,
        push: bool,
    ) {
        if !vandalism && self.matched_in_current && self.outbound.last() != Some(revision_curr) {
            if push && Some(&self.latest_revision) != revision_prev {
                self.inbound.push(revision_curr.clone());
            }
            self.latest_revision = revision_curr.clone();
        }
    }

    fn maybe_push_outbound(&mut self, revision_curr: &RevisionPointer) {
        if !self.matched_in_current {
            self.outbound.push(revision_curr.clone());
        }
    }
}

#[derive(Default)]
pub(crate) struct PageAnalysisInternals {
    options: PageAnalysisOptions,

    paragraphs_ht: FxHashMap<blake3::Hash, Vec<ParagraphPointer>>, // Hash table of paragraphs of all revisions
    sentences_ht: FxHashMap<blake3::Hash, Vec<SentencePointer>>, // Hash table of sentences of all revisions
    spam_hashes: FxHashSet<RevisionHash>, // Hashes of spam revisions; RevisionHash can be a SHA1 hash or a BLAKE3 hash but we expect all hashes in this revision to be of the same type

    revision_prev: Option<RevisionPointer>,
    // text_curr: String, /* pass text_curr as parameter instead */
    // temp: Vec<String>, /* replaced by disambiguate_* in analyse_page */
    scratch_buffers: (String, String),
}

// Spam detection variables.
// use f64 instead of f32 to replicate the behavior of the Python script
const CHANGE_PERCENTAGE: f64 = -0.40;
const PREVIOUS_LENGTH: usize = 1000;
const CURR_LENGTH: usize = 1000;
const UNMATCHED_PARAGRAPH: f64 = 0.0;
const TOKEN_DENSITY_LIMIT: f64 = 20.0;

// since the handling of paragraphs and sentences is almost identical, we generalize
trait ParasentPointer: Sized + Pointer {
    type ParentPointer: Pointer;
    const IS_SENTENCE: bool;

    fn allocate_new_in_parent(
        analysis: &mut PageAnalysis,
        parent: &Self::ParentPointer,
        text: ArcSubstring,
    ) -> Self;

    fn iterate_words(
        analysis: &mut PageAnalysis,
        parasents: &[Self],
        f: impl FnMut(&mut WordAnalysis),
    );
    fn all_parasents_in_parents(
        analysis: &mut PageAnalysis,
        prevs: &[Self::ParentPointer],
    ) -> Vec<Self>;
    fn find_in_parents(
        analysis: &mut PageAnalysis,
        prevs: &[Self::ParentPointer],
        hash: &blake3::Hash,
    ) -> Vec<Self>;
    fn store_in_parent(&self, analysis: &mut PageAnalysis, curr: &Self::ParentPointer);
    fn find_in_any_previous_revision(analysis: &mut PageAnalysis, hash: &blake3::Hash)
        -> Vec<Self>;

    fn split_into_parasents<'a>(
        parasent_text: &'a str,
        scratch_buffers: (&mut String, &mut String),
    ) -> Vec<Cow<'a, str>>;

    fn mark_all_children_matched(&self, analysis: &mut PageAnalysis);

    fn matched_in_current(&self, analysis: &mut PageAnalysis) -> bool;
    fn set_matched_in_current(&self, analysis: &mut PageAnalysis, value: bool);
}

impl ParasentPointer for ParagraphPointer {
    type ParentPointer = RevisionPointer;
    const IS_SENTENCE: bool = false;

    fn allocate_new_in_parent(
        analysis: &mut PageAnalysis,
        parent: &RevisionPointer,
        text: ArcSubstring,
    ) -> Self {
        let paragraph_pointer = analysis.new_paragraph(ParagraphImmutables::new(text));

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
        analysis: &mut PageAnalysis,
        paragraphs: &[Self],
        f: impl FnMut(&mut WordAnalysis),
    ) {
        analysis.iterate_words_in_paragraphs(paragraphs, f);
    }

    fn all_parasents_in_parents(
        analysis: &mut PageAnalysis,
        prevs: &[RevisionPointer],
    ) -> Vec<Self> {
        let mut result = Vec::new();
        for revision_prev in prevs {
            result.extend_from_slice(&analysis.revisions[revision_prev.0].paragraphs_ordered);
        }
        result
    }

    fn split_into_parasents<'a>(
        revision_text: &'a str,
        scratch_buffers: (&mut String, &mut String),
    ) -> Vec<Cow<'a, str>> {
        // Split the text of the current revision into paragraphs.
        let paragraphs = split_into_paragraphs(revision_text, scratch_buffers);
        paragraphs
            .into_iter()
            .map(trim_in_place)
            .filter(|s| !s.is_empty()) /* don't track empty paragraphs */
            .collect()
    }

    fn find_in_parents(
        analysis: &mut PageAnalysis,
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

    fn store_in_parent(&self, analysis: &mut PageAnalysis, curr: &Self::ParentPointer) {
        let revision_curr = &mut analysis.revisions[curr.0];
        revision_curr
            .paragraphs_by_hash
            .entry(self.hash_value)
            .and_modify(|v| v.push(self.clone()))
            .or_insert_with(|| MaybeVec::new_single(self.clone()));
        revision_curr.paragraphs_ordered.push(self.clone());
    }

    fn find_in_any_previous_revision(
        analysis: &mut PageAnalysis,
        hash: &blake3::Hash,
    ) -> Vec<Self> {
        analysis
            .internals
            .paragraphs_ht
            .get(hash)
            .cloned()
            .unwrap_or_default()
    }

    fn mark_all_children_matched(&self, analysis: &mut PageAnalysis) {
        for sentence in &analysis.paragraphs[self.0].sentences_ordered {
            analysis.sentences[sentence.0].matched_in_current = true;
            for word in &analysis.sentences[sentence.0].words_ordered {
                analysis.word_analyses[word.0].matched_in_current = true;
            }
        }
    }

    fn matched_in_current(&self, analysis: &mut PageAnalysis) -> bool {
        analysis.paragraphs[self.0].matched_in_current
    }

    fn set_matched_in_current(&self, analysis: &mut PageAnalysis, value: bool) {
        analysis.paragraphs[self.0].matched_in_current = value;
    }
}

impl ParasentPointer for SentencePointer {
    type ParentPointer = ParagraphPointer;
    const IS_SENTENCE: bool = true;

    fn allocate_new_in_parent(
        analysis: &mut PageAnalysis,
        parent: &ParagraphPointer,
        text: ArcSubstring,
    ) -> Self {
        let sentence_pointer = analysis.new_sentence(SentenceImmutables::new(text));

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
        analysis: &mut PageAnalysis,
        sentences: &[Self],
        f: impl FnMut(&mut WordAnalysis),
    ) {
        analysis.iterate_words_in_sentences(sentences, f);
    }

    fn all_parasents_in_parents(
        analysis: &mut PageAnalysis,
        prevs: &[ParagraphPointer],
    ) -> Vec<Self> {
        let mut result = Vec::new();
        for paragraph_prev in prevs {
            result.extend_from_slice(&analysis.paragraphs[paragraph_prev.0].sentences_ordered);
        }
        result
    }

    fn split_into_parasents<'a>(
        paragraph_text: &'a str,
        scratch_buffers: (&mut String, &mut String),
    ) -> Vec<Cow<'a, str>> {
        // Split the current paragraph into sentences.
        let sentences = split_into_sentences(paragraph_text, scratch_buffers);
        sentences
            .into_iter()
            .map(trim_in_place)
            .filter(|s| !s.is_empty()) /* don't track empty sentences */
            .map(|s| {
                let cleaned_string = split_into_tokens(&s).join(" ");
                if cleaned_string != s {
                    Cow::Owned(cleaned_string)
                } else {
                    s
                }
            }) /* here whitespaces in the sentence are cleaned */
            .collect()
    }

    fn find_in_parents(
        analysis: &mut PageAnalysis,
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

    fn store_in_parent(&self, analysis: &mut PageAnalysis, curr: &Self::ParentPointer) {
        let paragraph_curr = &mut analysis.paragraphs[curr.0];
        paragraph_curr
            .sentences_by_hash
            .entry(self.hash_value)
            .and_modify(|v| v.push(self.clone()))
            .or_insert_with(|| MaybeVec::new_single(self.clone()));
        paragraph_curr.sentences_ordered.push(self.clone());
    }

    fn find_in_any_previous_revision(
        analysis: &mut PageAnalysis,
        hash: &blake3::Hash,
    ) -> Vec<Self> {
        analysis
            .internals
            .sentences_ht
            .get(hash)
            .cloned()
            .unwrap_or_default()
    }

    fn mark_all_children_matched(&self, analysis: &mut PageAnalysis) {
        for word in &analysis.sentences[self.0].words_ordered {
            analysis.word_analyses[word.0].matched_in_current = true;
        }
    }

    fn matched_in_current(&self, analysis: &mut PageAnalysis) -> bool {
        analysis.sentences[self.0].matched_in_current
    }

    fn set_matched_in_current(&self, analysis: &mut PageAnalysis, value: bool) {
        analysis.sentences[self.0].matched_in_current = value;
    }
}

#[derive(Default, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct PageAnalysisOptions {
    /// Use optimized lowercasing algorithm that is faster than default for inputs with <= 90% ASCII content.
    #[cfg(feature = "optimized-lowercase")]
    pub optimize_non_ascii: bool,
    /// Use the original Python stdlib diff algorithm by invoking Python with pyo3.
    ///
    /// Multi-threading may be significantly slower than in pure-Rust due to global interpreter lock (GIL) contention.
    #[cfg(feature = "python-diff")]
    pub use_python_diff: bool,
    // optimized-str is absolutely better in performance, the only downside is more dependencies,
    // so we provide no runtime switch since cargo feature merging in dependency trees should be fine
}

impl PageAnalysisOptions {
    pub const fn new() -> Self {
        Self {
            #[cfg(feature = "optimized-lowercase")]
            optimize_non_ascii: false,
            #[cfg(feature = "python-diff")]
            use_python_diff: false,
        }
    }

    #[cfg(feature = "optimized-lowercase")]
    pub const fn optimize_non_ascii(mut self) -> Self {
        self.optimize_non_ascii = true;
        self
    }

    #[cfg(feature = "python-diff")]
    pub const fn use_python_diff(mut self) -> Self {
        self.use_python_diff = true;
        self
    }
}

impl PageAnalysis {
    /// Runs the WikiWho authorship analysis on an ordered sequence of revisions.
    ///
    /// This is the main entry point for the algorithm. It processes revisions from
    /// oldest to newest, performing spam/vandalism detection and building a
    /// token-level authorship graph.
    ///
    /// `xml_revisions` must be in chronological order (oldest first), as returned
    /// by [`DumpParser::parse_page`](crate::dump_parser::DumpParser::parse_page).
    ///
    /// # Errors
    ///
    /// Returns [`AnalysisError::NoValidRevisions`] if every revision in the input
    /// is classified as spam or has empty/deleted text.
    pub fn analyse_page<I, R>(xml_revisions: I) -> Result<Self, AnalysisError>
    where
        R: Borrow<Revision>,
        I: IntoIterator<Item = R>,
    {
        Self::analyse_page_with_options(xml_revisions, PageAnalysisOptions::default())
    }

    /// Like [`analyse_page`](Self::analyse_page), but lets you select algorithm
    /// behavior via [`PageAnalysisOptions`].
    ///
    /// Use this entry point when you need non-default behavior, such as the
    /// Python-compatible diff algorithm (`python-diff` feature) or the optimized
    /// non-ASCII lowercasing path (`optimized-lowercase` feature). See
    /// [`PageAnalysisOptions`] for the full list of options;
    /// [`analyse_page`](Self::analyse_page) is exactly this function called with
    /// [`PageAnalysisOptions::default`].
    ///
    /// As with [`analyse_page`](Self::analyse_page), `xml_revisions` must be in
    /// chronological order (oldest first), as returned by
    /// [`DumpParser::parse_page`](crate::dump_parser::DumpParser::parse_page).
    ///
    /// # Errors
    ///
    /// Returns [`AnalysisError::NoValidRevisions`] if every revision in the input
    /// is classified as spam or has empty/deleted text.
    pub fn analyse_page_with_options<I, R>(
        xml_revisions: I,
        analysis_options: PageAnalysisOptions,
    ) -> Result<Self, AnalysisError>
    where
        R: Borrow<Revision>,
        I: IntoIterator<Item = R>,
    {
        // This means we'll always have an unreferenced dummy revision in the revisions array at index 0,
        // which is not ideal but simplifies the implementation and data model significantly.
        let initial_revision = (RevisionAnalysis::default(), RevisionImmutables::dummy()); /* will be overwritten before being read */
        let mut analysis = PageAnalysis::new(initial_revision);
        analysis.internals.options = analysis_options;

        let mut at_least_one = false;

        // Iterate over revisions of the article.
        // Analysis begins at the oldest revision and progresses to the newest.
        for xml_revision_source in xml_revisions {
            let xml_revision = xml_revision_source.borrow();

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

            let revision_data =
                RevisionImmutables::from_revision_with_options(xml_revision, analysis_options);
            let mut vandalism = false;

            if analysis.internals.spam_hashes.contains(&rev_hash) {
                // The content of this revision has already been marked as spam
                vandalism = true;
            }

            // Spam detection: Deletion
            // On initial revision this resolves to a no-op, since length_lowercase is 0
            if !(vandalism || xml_revision.comment.is_some() && xml_revision.minor) {
                let revision_prev = &analysis.current_revision; /* !! since we have not yet updated current_revision, this is the previous revision */
                let change_percentage = (revision_data.length_lowercase as f64
                    - revision_prev.length_lowercase as f64)
                    / revision_prev.length_lowercase as f64;

                if revision_prev.length_lowercase > PREVIOUS_LENGTH
                    && revision_data.length_lowercase < CURR_LENGTH
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
            let mut revision_pointer = analysis.new_revision(revision_data);

            // Update the information about the previous revision.
            std::mem::swap(&mut analysis.current_revision, &mut revision_pointer);
            if at_least_one {
                analysis.internals.revision_prev = Some(revision_pointer);
            } /* if !at_least_one we do not yet have any valid revision (revision_pointer contains a
              dummy value or vandalism revision) to refer to as previous, so the previous revision is discarded */

            // Perform the actual word (aka. token) matching
            vandalism = analysis.determine_authorship();

            if vandalism {
                // Skip this revision due to vandalism
                if at_least_one {
                    // Revert the state of `revision_curr` to the beginning of the loop iteration
                    analysis.current_revision =
                        analysis.internals.revision_prev.take().expect(
                            "should not have been deleted in the call to determine_authorship",
                        );
                } /* while !at_least_one we expect revision_prev to be None */

                // Mark the revision as spam
                analysis.spam_ids.push(xml_revision.id);
                analysis.internals.spam_hashes.insert(rev_hash);
            } else {
                // Store the current revision in the result
                analysis
                    .ordered_revisions
                    .push(analysis.current_revision.clone());
                analysis.revisions_by_id.insert(
                    analysis.current_revision.id,
                    analysis.current_revision.clone(),
                );

                // and note that we have processed at least one valid revision
                at_least_one = true;
            }

            // we explicitely drop this iteration source object before getting the next one
            // so we can potentially free unused memory
            drop(xml_revision_source);
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
                f(&mut self.word_analyses[word.0]);
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
                    f(&mut self.word_analyses[word.0]);
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
        let revision_curr = self.current_revision.clone(); /* short-hand */
        let revision_prev = self.internals.revision_prev.clone(); /* short-hand */

        let mut unmatched_sentences_curr = Vec::new();
        let mut unmatched_sentences_prev = Vec::new();

        let mut matched_sentences_prev = Vec::new();
        let mut matched_words_prev = Vec::new();

        let mut possible_vandalism = false;
        let mut vandalism = false;

        // Analysis of the paragraphs in the current revision
        let (unmatched_paragraphs_curr, unmatched_paragraphs_prev, matched_paragraphs_prev, _) =
            self.analyse_parasents_in_revgraph(
                #[allow(clippy::cloned_ref_to_slice_refs)]
                // clone is needed to a avoid borrow conflict
                &[self.current_revision.clone()],
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
                / self[&self.current_revision].paragraphs_ordered.len() as f64
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
                word.maybe_push_outbound(&revision_curr)
            });

            // ???
            if unmatched_sentences_prev.is_empty() {
                self.iterate_words_in_paragraphs(&unmatched_paragraphs_prev, |word| {
                    word.maybe_push_outbound(&revision_curr)
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
            word.maybe_push_inbound(
                vandalism,
                &revision_curr,
                revision_prev.as_ref(),
                push_inbound,
            );
            // then reset the matched status
            word.matched_in_current = false;
        };

        for matched_paragraph in &matched_paragraphs_prev {
            matched_paragraph.set_matched_in_current(self, false);
            for matched_sentence in &self.paragraphs[matched_paragraph.0].sentences_ordered {
                self.sentences[matched_sentence.0].matched_in_current = false;

                for matched_word in &self.sentences[matched_sentence.0].words_ordered {
                    handle_word(&mut self.word_analyses[matched_word.0], true);
                }
            }
        }
        for matched_sentence in &matched_sentences_prev {
            matched_sentence.set_matched_in_current(self, false);

            for matched_word in &self.sentences[matched_sentence.0].words_ordered {
                handle_word(&mut self.word_analyses[matched_word.0], true);
            }
        }
        for matched_word in &matched_words_prev {
            // there is no inbound chance because we only diff with words of previous revision -> push_inbound = false
            handle_word(&mut self.word_analyses[matched_word.0], false);
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

            P::iterate_words(self, std::slice::from_ref(parasent_prev_pointer), |word| {
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
                    let paragraph_pointer = P::allocate_new_in_parent(
                        self,
                        parasent_curr_pointer,
                        parasent_curr_pointer
                            .value()
                            .reattach_substring(parasent_text),
                    );
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

        let mut token_to_revsubstr = HashMap::new();

        // Split sentences into words.
        let mut text_prev = Vec::new();
        for sentence_prev_pointer in unmatched_sentences_prev {
            let sentence_prev = &self.sentences[sentence_prev_pointer.0];
            for word_prev_pointer in &sentence_prev.words_ordered {
                if !self.word_analyses[word_prev_pointer.0].matched_in_current {
                    let interned = interner.intern(word_prev_pointer.value().clone());
                    text_prev.push(interned);
                    unmatched_words_prev.push((interned, word_prev_pointer.clone()));
                    token_to_revsubstr.insert(interned, word_prev_pointer.value());
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
                .map(|s| {
                    interner.intern(sentence_curr_pointer.value().reattach_substring(s.into()))
                })
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
            analysis: &mut PageAnalysis,
            word: ArcSubstring,
            sentence_pointer: &SentencePointer,
        ) {
            let word_pointer = analysis.new_word(
                WordImmutables::new(word),
                WordAnalysis::new(&analysis.current_revision),
            );

            analysis.words.push(word_pointer.clone());
            analysis.sentences[sentence_pointer.0]
                .words_ordered
                .push(word_pointer);
            analysis.revisions[analysis.current_revision.0].original_adds += 1;
        }

        // Edit consists of adding new content, not changing/removing content
        if text_prev.is_empty() {
            for (i, sentence_curr_pointer) in unmatched_sentences_curr.iter().enumerate() {
                for word_interned in unmatched_sentence_curr_splitted[i].iter() {
                    allocate_new_word(
                        self,
                        interner[*word_interned].clone(),
                        sentence_curr_pointer,
                    );
                }
            }
            return (matched_words_prev, false);
        }

        // do the diffing!
        let mut diff: Vec<_>;
        #[cfg(feature = "python-diff")]
        {
            if self.internals.options.use_python_diff {
                diff = utils::python_diff(&text_prev, &text_curr, &mut interner);
            } else {
                diff = utils::difflib_diff(&text_prev, &text_curr);
            }
        }
        #[cfg(not(feature = "python-diff"))]
        {
            diff = utils::difflib_diff(&text_prev, &text_curr);
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
                                            && !self.word_analyses[w_pointer.0].matched_in_current
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
                                            && !self.word_analyses[w_pointer.0].matched_in_current
                                    })
                                {
                                    self[word_prev].matched_in_current = true;

                                    let revision_curr = self.current_revision.clone(); /* need to clone first, otherwise borrow-checker complains */
                                    self[word_prev].outbound.push(revision_curr);

                                    matched_words_prev.push(word_prev.clone());
                                    *change = None;
                                }
                            }
                            ChangeTag::Insert => {
                                // a new added word
                                curr_matched = true;

                                allocate_new_word(
                                    self,
                                    interner[*word_interned].clone(),
                                    sentence_curr,
                                );

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
                    allocate_new_word(self, interner[*word_interned].clone(), sentence_curr);
                }
            }
        }

        (matched_words_prev, false)
    }
}
