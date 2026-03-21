// SPDX-License-Identifier: MIT AND MPL-2.0
use std::{collections::HashMap, sync::Arc};

use rustc_hash::FxHashMap;

use crate::algorithm::ParagraphPointer;

use super::{
    PageAnalysis, PageAnalysisInternals, ParagraphAnalysis, ParagraphImmutables, RevisionAnalysis,
    RevisionImmutables, RevisionPointer, SentenceAnalysis, SentenceImmutables, SentencePointer,
    WordAnalysis, WordImmutables, WordPointer,
};

// ---------------------------------------------------------------------------
// Intermediate serialized types — not exported, only used for serde conversion
// ---------------------------------------------------------------------------

#[derive(serde::Serialize, serde::Deserialize)]
struct SerializedWordAnalysis {
    origin_revision: usize,
    latest_revision: usize,
    inbound: Vec<usize>,
    outbound: Vec<usize>,
}

#[derive(serde::Serialize, serde::Deserialize)]
struct SerializedSentenceAnalysis {
    words_ordered: Vec<usize>,
}

#[derive(serde::Serialize, serde::Deserialize)]
struct SerializedParagraphAnalysis {
    sentences_ordered: Vec<usize>,
}

#[derive(serde::Serialize, serde::Deserialize)]
struct SerializedRevisionAnalysis {
    paragraphs_ordered: Vec<usize>,
    original_adds: usize,
}

#[derive(serde::Serialize, serde::Deserialize)]
struct SerializedPageAnalysis {
    // Immutables arrays first — serialised as plain values, not Arc wrappers
    revision_immutables: Vec<RevisionImmutables>,
    paragraph_immutables: Vec<ParagraphImmutables>,
    sentence_immutables: Vec<SentenceImmutables>,
    word_immutables: Vec<super::WordImmutables>,
    // Analysis arrays with usize pointer indices
    revisions: Vec<SerializedRevisionAnalysis>,
    paragraphs: Vec<SerializedParagraphAnalysis>,
    sentences: Vec<SerializedSentenceAnalysis>,
    word_analyses: Vec<SerializedWordAnalysis>,
    // Public pointer fields as indices
    spam_ids: Vec<i32>,
    revisions_by_id: HashMap<i32, usize>,
    ordered_revisions: Vec<usize>,
    words: Vec<usize>,
    current_revision: usize,
}

// ---------------------------------------------------------------------------
// Serialize
// ---------------------------------------------------------------------------

impl serde::Serialize for PageAnalysis {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let serialized = SerializedPageAnalysis {
            revision_immutables: self
                .revision_immutables
                .iter()
                .map(|arc| (**arc).clone())
                .collect(),
            paragraph_immutables: self
                .paragraph_immutables
                .iter()
                .map(|arc| (**arc).clone())
                .collect(),
            sentence_immutables: self
                .sentence_immutables
                .iter()
                .map(|arc| (**arc).clone())
                .collect(),
            word_immutables: self
                .word_immutables
                .iter()
                .map(|arc| (**arc).clone())
                .collect(),
            revisions: self
                .revisions
                .iter()
                .map(|r| SerializedRevisionAnalysis {
                    paragraphs_ordered: r.paragraphs_ordered.iter().map(|p| p.0).collect(),
                    original_adds: r.original_adds,
                })
                .collect(),
            paragraphs: self
                .paragraphs
                .iter()
                .map(|p| SerializedParagraphAnalysis {
                    sentences_ordered: p.sentences_ordered.iter().map(|s| s.0).collect(),
                })
                .collect(),
            sentences: self
                .sentences
                .iter()
                .map(|s| SerializedSentenceAnalysis {
                    words_ordered: s.words_ordered.iter().map(|w| w.0).collect(),
                })
                .collect(),
            word_analyses: self
                .word_analyses
                .iter()
                .map(|w| SerializedWordAnalysis {
                    origin_revision: w.origin_revision.0,
                    latest_revision: w.latest_revision.0,
                    inbound: w.inbound.iter().map(|r| r.0).collect(),
                    outbound: w.outbound.iter().map(|r| r.0).collect(),
                })
                .collect(),
            spam_ids: self.spam_ids.clone(),
            revisions_by_id: self
                .revisions_by_id
                .iter()
                .map(|(&id, ptr)| (id, ptr.0))
                .collect(),
            ordered_revisions: self.ordered_revisions.iter().map(|r| r.0).collect(),
            words: self.words.iter().map(|w| w.0).collect(),
            current_revision: self.current_revision.0,
        };
        serialized.serialize(serializer)
    }
}

// ---------------------------------------------------------------------------
// Deserialize
// ---------------------------------------------------------------------------

impl<'de> serde::Deserialize<'de> for PageAnalysis {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let s = SerializedPageAnalysis::deserialize(deserializer)?;

        // Build Arc arrays from the deserialized plain values.
        // ParagraphImmutables and SentenceImmutables have custom Deserialize impls
        // that reconstruct blake3::Hash from `value`, so these Arcs are fully valid.
        let revision_arcs: Vec<Arc<RevisionImmutables>> =
            s.revision_immutables.into_iter().map(Arc::new).collect();
        let paragraph_arcs: Vec<Arc<ParagraphImmutables>> =
            s.paragraph_immutables.into_iter().map(Arc::new).collect();
        let sentence_arcs: Vec<Arc<SentenceImmutables>> =
            s.sentence_immutables.into_iter().map(Arc::new).collect();
        let word_arcs: Vec<Arc<WordImmutables>> =
            s.word_immutables.into_iter().map(Arc::new).collect();

        // Helper closures — validate index bounds and reconstruct pointers.
        // All pointers for the same index share the single Arc heap allocation.
        let rev_ptr = |idx: usize| -> Result<RevisionPointer, D::Error> {
            revision_arcs
                .get(idx)
                .ok_or_else(|| {
                    serde::de::Error::custom(format!(
                        "revision index {idx} out of bounds (len={})",
                        revision_arcs.len()
                    ))
                })
                .map(|arc| RevisionPointer(idx, arc.clone()))
        };
        let par_ptr = |idx: usize| -> Result<ParagraphPointer, D::Error> {
            paragraph_arcs
                .get(idx)
                .ok_or_else(|| {
                    serde::de::Error::custom(format!(
                        "paragraph index {idx} out of bounds (len={})",
                        paragraph_arcs.len()
                    ))
                })
                .map(|arc| ParagraphPointer(idx, arc.clone()))
        };
        let sent_ptr = |idx: usize| -> Result<SentencePointer, D::Error> {
            sentence_arcs
                .get(idx)
                .ok_or_else(|| {
                    serde::de::Error::custom(format!(
                        "sentence index {idx} out of bounds (len={})",
                        sentence_arcs.len()
                    ))
                })
                .map(|arc| SentencePointer(idx, arc.clone()))
        };
        let word_ptr = |idx: usize| -> Result<WordPointer, D::Error> {
            word_arcs
                .get(idx)
                .ok_or_else(|| {
                    serde::de::Error::custom(format!(
                        "word index {idx} out of bounds (len={})",
                        word_arcs.len()
                    ))
                })
                .map(|arc| WordPointer(idx, arc.clone()))
        };

        // Reconstruct revisions; transient fields are zeroed.
        let revisions = s
            .revisions
            .into_iter()
            .map(|r| {
                let paragraphs_ordered = r
                    .paragraphs_ordered
                    .into_iter()
                    .map(|idx| par_ptr(idx))
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(RevisionAnalysis {
                    paragraphs_by_hash: FxHashMap::default(),
                    paragraphs_ordered,
                    original_adds: r.original_adds,
                })
            })
            .collect::<Result<Vec<_>, D::Error>>()?;

        // Reconstruct paragraphs; transient fields are zeroed.
        let paragraphs = s
            .paragraphs
            .into_iter()
            .map(|p| {
                let sentences_ordered = p
                    .sentences_ordered
                    .into_iter()
                    .map(|idx| sent_ptr(idx))
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(ParagraphAnalysis {
                    sentences_by_hash: FxHashMap::default(),
                    sentences_ordered,
                    matched_in_current: false,
                })
            })
            .collect::<Result<Vec<_>, D::Error>>()?;

        // Reconstruct sentences.
        let sentences = s
            .sentences
            .into_iter()
            .map(|s| {
                let words_ordered = s
                    .words_ordered
                    .into_iter()
                    .map(|idx| word_ptr(idx))
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(SentenceAnalysis {
                    words_ordered,
                    matched_in_current: false,
                })
            })
            .collect::<Result<Vec<_>, D::Error>>()?;

        // Reconstruct words.
        let word_analyses = s
            .word_analyses
            .into_iter()
            .map(|w| {
                Ok(WordAnalysis {
                    origin_revision: rev_ptr(w.origin_revision)?,
                    latest_revision: rev_ptr(w.latest_revision)?,
                    matched_in_current: false,
                    inbound: w
                        .inbound
                        .into_iter()
                        .map(|idx| rev_ptr(idx))
                        .collect::<Result<Vec<_>, _>>()?,
                    outbound: w
                        .outbound
                        .into_iter()
                        .map(|idx| rev_ptr(idx))
                        .collect::<Result<Vec<_>, _>>()?,
                })
            })
            .collect::<Result<Vec<_>, D::Error>>()?;

        // Reconstruct public pointer fields.
        let revisions_by_id = s
            .revisions_by_id
            .into_iter()
            .map(|(id, idx)| rev_ptr(idx).map(|ptr| (id, ptr)))
            .collect::<Result<HashMap<_, _>, _>>()?;
        let ordered_revisions = s
            .ordered_revisions
            .into_iter()
            .map(|idx| rev_ptr(idx))
            .collect::<Result<Vec<_>, _>>()?;
        let words = s
            .words
            .into_iter()
            .map(|idx| word_ptr(idx))
            .collect::<Result<Vec<_>, _>>()?;
        let current_revision = rev_ptr(s.current_revision)?;

        Ok(PageAnalysis {
            revisions,
            revision_immutables: revision_arcs,
            paragraphs,
            paragraph_immutables: paragraph_arcs,
            sentences,
            sentence_immutables: sentence_arcs,
            word_analyses,
            word_immutables: word_arcs,
            spam_ids: s.spam_ids,
            revisions_by_id,
            ordered_revisions,
            words,
            current_revision,
            internals: PageAnalysisInternals::default(),
        })
    }
}
