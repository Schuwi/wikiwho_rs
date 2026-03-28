// SPDX-License-Identifier: MPL-2.0
use std::{collections::HashMap, ops::Range, sync::Arc};

use rustc_hash::FxHashMap;

use crate::algorithm::{ArcSubstring, ParagraphPointer};

use super::{
    PageAnalysis, PageAnalysisInternals, ParagraphAnalysis, ParagraphImmutables, RevisionAnalysis,
    RevisionImmutables, RevisionPointer, SentenceAnalysis, SentenceImmutables, SentencePointer,
    WordAnalysis, WordImmutables, WordPointer,
};

// ---------------------------------------------------------------------------
// Intermediate serialized types — not exported, only used for serde conversion
// ---------------------------------------------------------------------------

#[derive(serde::Serialize, serde::Deserialize)]
struct SerializedArcSubstring {
    source_index: usize,
    source_range: Range<usize>,
}

#[derive(serde::Serialize, serde::Deserialize)]
struct SerializedRevisionImmutables {
    id: i32,
    length_lowercase: usize,
    text_lowercase: SerializedArcSubstring,
}

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
    #[serde(with = "vec_arc_string")]
    source_strings: Vec<Arc<String>>,
    // Immutables arrays first — serialised as plain values, not Arc wrappers
    revision_immutables: Vec<SerializedRevisionImmutables>,
    paragraph_immutables: Vec<SerializedArcSubstring>,
    sentence_immutables: Vec<SerializedArcSubstring>,
    word_immutables: Vec<SerializedArcSubstring>,
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

mod vec_arc_string {
    use std::sync::Arc;

    pub fn serialize<S: serde::Serializer>(obj: &Vec<Arc<String>>, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_seq(obj.iter().map(|s| s.as_str()))
    }

    pub fn deserialize<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<Vec<Arc<String>>, D::Error> {
        let strings: Vec<String> = serde::Deserialize::deserialize(deserializer)?;
        Ok(strings.into_iter().map(|s| Arc::new(s)).collect())
    }
}

// ---------------------------------------------------------------------------
// Serialize
// ---------------------------------------------------------------------------

impl serde::Serialize for PageAnalysis {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut source_strings = Vec::new();

        let mut arc_string_lookup = HashMap::new();
        let mut arc_index = |base_arc: &Arc<String>| {
            *arc_string_lookup
                .entry(Arc::as_ptr(base_arc))
                .or_insert_with(|| {
                    let index = source_strings.len();
                    source_strings.push(base_arc.clone());
                    index
                })
        };
        let mut serialize_arc_substr = |arc_substr: &ArcSubstring| {
            let index = arc_index(arc_substr.base_string());

            let base_bytestr = arc_substr.base_string().as_bytes();
            let substr_bytestr = arc_substr.as_str().as_bytes();

            let (substr_start, substr_end) = if substr_bytestr.is_empty() {
                (0, 0)
            } else {
                let start = base_bytestr.element_offset(&substr_bytestr[0]).expect(
                    "ArcSubstring::as_str to be a reference inside ArcSubstring::base_string",
                );
                (start, start + substr_bytestr.len())
            };

            SerializedArcSubstring {
                source_index: index,
                source_range: substr_start..substr_end,
            }
        };

        let revision_immutables = self
            .revision_immutables
            .iter()
            .map(|rev| SerializedRevisionImmutables {
                id: rev.id,
                length_lowercase: rev.length_lowercase,
                text_lowercase: serialize_arc_substr(&rev.text_lowercase),
            })
            .collect();
        let paragraph_immutables = self
            .paragraph_immutables
            .iter()
            .map(|e| serialize_arc_substr(&e.value))
            .collect();
        let sentence_immutables = self
            .sentence_immutables
            .iter()
            .map(|e| serialize_arc_substr(&e.value))
            .collect();
        let word_immutables = self
            .word_immutables
            .iter()
            .map(|e| serialize_arc_substr(&e.value))
            .collect();

        let serialized = SerializedPageAnalysis {
            source_strings,
            revision_immutables,
            paragraph_immutables,
            sentence_immutables,
            word_immutables,
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
        
        let source_strings = s.source_strings;
        let deserialize_substr = |serial_substr: SerializedArcSubstring| {
            if let Some(source_string) = source_strings.get(serial_substr.source_index) {
                if let Some(substr) = source_string.get(serial_substr.source_range) {
                    Ok(ArcSubstring::new_substr(source_string.clone(), substr))
                } else {
                    Err(serde::de::Error::custom(format!(
                        "substring range {} out of bounds (source len={}) or not on character boundary",
                        serial_substr.source_index,
                        source_strings.len()
                    )))
                }
            } else {
                Err(serde::de::Error::custom(format!(
                    "source string index {} out of bounds (len={})",
                    serial_substr.source_index,
                    source_strings.len()
                )))
            }
        };

        // Build Arc arrays from the deserialized plain values.
        // ParagraphImmutables and SentenceImmutables have custom Deserialize impls
        // that reconstruct blake3::Hash from `value`, so these Arcs are fully valid.
        let revision_arcs: Vec<Arc<RevisionImmutables>> = s
            .revision_immutables
            .into_iter()
            .map(|rev| {
                Ok(Arc::new(RevisionImmutables {
                    id: rev.id,
                    length_lowercase: rev.length_lowercase,
                    text_lowercase: deserialize_substr(rev.text_lowercase)?,
                }))
            })
            .collect::<Result<Vec<_>, D::Error>>()?;
        let paragraph_arcs: Vec<Arc<ParagraphImmutables>> = s
            .paragraph_immutables
            .into_iter()
            .map(|s| Ok(Arc::new(ParagraphImmutables::new(deserialize_substr(s)?))))
            .collect::<Result<Vec<_>, D::Error>>()?;
        let sentence_arcs: Vec<Arc<SentenceImmutables>> = s
            .sentence_immutables
            .into_iter()
            .map(|s| Ok(Arc::new(SentenceImmutables::new(deserialize_substr(s)?))))
            .collect::<Result<Vec<_>, D::Error>>()?;
        let word_arcs: Vec<Arc<WordImmutables>> = s
            .word_immutables
            .into_iter()
            .map(|s| Ok(Arc::new(WordImmutables::new(deserialize_substr(s)?))))
            .collect::<Result<Vec<_>, D::Error>>()?;

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
                    .map(&par_ptr)
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
                    .map(&sent_ptr)
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
                    .map(&word_ptr)
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
                        .map(&rev_ptr)
                        .collect::<Result<Vec<_>, _>>()?,
                    outbound: w
                        .outbound
                        .into_iter()
                        .map(&rev_ptr)
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
            .map(&rev_ptr)
            .collect::<Result<Vec<_>, _>>()?;
        let words = s
            .words
            .into_iter()
            .map(word_ptr)
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dump_parser::{Contributor, Revision, Text};
    use compact_str::CompactString;
    use std::sync::Arc;

    fn make_revision(id: i32, text: &str) -> Revision {
        Revision {
            id,
            timestamp: chrono::DateTime::from_timestamp_nanos(1_700_000_000_000_000_000),
            contributor: Contributor {
                id: Some(id),
                username: CompactString::from(format!("User{id}")),
            },
            text: Text::Normal(text.to_string()),
            sha1: None,
            comment: None,
            minor: false,
        }
    }

    /// Build a PageAnalysis with controlled structure:
    /// - dummy rev (idx 0), rev1 (idx 1), rev2 (idx 2)
    /// - 1 paragraph (idx 0) -> 1 sentence (idx 0) -> 2 words (idx 0, 1)
    /// - both words share origin_revision = rev1 (idx 1)
    /// - word0 has inbound=[rev2], word1 has outbound=[rev2]
    fn build_test_page_analysis() -> PageAnalysis {
        let rev1_imm =
            RevisionImmutables::from_revision(&make_revision(1, "Hello world. This is a test."));
        let rev2_imm =
            RevisionImmutables::from_revision(&make_revision(2, "Hello world. Different text."));

        let mut pa = PageAnalysis::new((RevisionAnalysis::default(), RevisionImmutables::dummy()));

        let rev1_ptr = pa.new_revision(rev1_imm);
        let rev2_ptr = pa.new_revision(rev2_imm);

        let par_ptr = pa.new_paragraph(ParagraphImmutables::new(ArcSubstring::new_source(
            Arc::new("Hello world.".to_string()),
        )));
        let sent_ptr = pa.new_sentence(SentenceImmutables::new(ArcSubstring::new_source(
            Arc::new("Hello world.".to_string()),
        )));

        let word0 = WordAnalysis {
            origin_revision: rev1_ptr.clone(),
            latest_revision: rev2_ptr.clone(),
            matched_in_current: false,
            inbound: vec![rev2_ptr.clone()],
            outbound: vec![],
        };
        let word0_ptr = pa.new_word(
            WordImmutables::new(ArcSubstring::new_source(Arc::new("hello".to_string()))),
            word0,
        );

        let word1 = WordAnalysis {
            origin_revision: rev1_ptr.clone(),
            latest_revision: rev1_ptr.clone(),
            matched_in_current: false,
            inbound: vec![],
            outbound: vec![rev2_ptr.clone()],
        };
        let word1_ptr = pa.new_word(
            WordImmutables::new(ArcSubstring::new_source(Arc::new("world".to_string()))),
            word1,
        );

        pa.sentences[sent_ptr.0].words_ordered = vec![word0_ptr.clone(), word1_ptr.clone()];
        pa.paragraphs[par_ptr.0].sentences_ordered = vec![sent_ptr];
        pa.revisions[rev1_ptr.0].paragraphs_ordered = vec![par_ptr];
        pa.revisions[rev1_ptr.0].original_adds = 2;

        pa.revisions_by_id.insert(1, rev1_ptr.clone());
        pa.revisions_by_id.insert(2, rev2_ptr.clone());
        pa.ordered_revisions = vec![rev1_ptr, rev2_ptr.clone()];
        pa.words = vec![word0_ptr, word1_ptr];
        pa.current_revision = rev2_ptr;
        pa.spam_ids = vec![99];

        pa
    }

    /// Field-by-field structural comparison (PageAnalysis has no PartialEq).
    fn assert_roundtrip_eq(orig: &PageAnalysis, deser: &PageAnalysis) {
        // Immutables counts
        assert_eq!(
            orig.revision_immutables.len(),
            deser.revision_immutables.len()
        );
        assert_eq!(
            orig.paragraph_immutables.len(),
            deser.paragraph_immutables.len()
        );
        assert_eq!(
            orig.sentence_immutables.len(),
            deser.sentence_immutables.len()
        );
        assert_eq!(orig.word_immutables.len(), deser.word_immutables.len());

        // Revision immutables
        for (i, (o, d)) in orig
            .revision_immutables
            .iter()
            .zip(&deser.revision_immutables)
            .enumerate()
        {
            assert_eq!(
                o.length_lowercase, d.length_lowercase,
                "revision_immutables[{i}].length_lowercase"
            );
            assert_eq!(
                o.text_lowercase, d.text_lowercase,
                "revision_immutables[{i}].text_lowercase"
            );
            assert_eq!(o.id, d.id, "revision_immutables[{i}].id");
        }

        // Paragraph immutables
        for (i, (o, d)) in orig
            .paragraph_immutables
            .iter()
            .zip(&deser.paragraph_immutables)
            .enumerate()
        {
            assert_eq!(o.value, d.value, "paragraph_immutables[{i}].value");
        }

        // Sentence immutables
        for (i, (o, d)) in orig
            .sentence_immutables
            .iter()
            .zip(&deser.sentence_immutables)
            .enumerate()
        {
            assert_eq!(o.value, d.value, "sentence_immutables[{i}].value");
        }

        // Word immutables
        for (i, (o, d)) in orig
            .word_immutables
            .iter()
            .zip(&deser.word_immutables)
            .enumerate()
        {
            assert_eq!(o.value, d.value, "word_immutables[{i}].value");
        }

        // Revisions analysis
        assert_eq!(orig.revisions.len(), deser.revisions.len());
        for (i, (o, d)) in orig.revisions.iter().zip(&deser.revisions).enumerate() {
            assert_eq!(
                o.original_adds, d.original_adds,
                "revisions[{i}].original_adds"
            );
            assert_eq!(
                o.paragraphs_ordered.len(),
                d.paragraphs_ordered.len(),
                "revisions[{i}].paragraphs_ordered.len"
            );
            for (j, (op, dp)) in o
                .paragraphs_ordered
                .iter()
                .zip(&d.paragraphs_ordered)
                .enumerate()
            {
                assert_eq!(op.0, dp.0, "revisions[{i}].paragraphs_ordered[{j}]");
            }
        }

        // Paragraphs analysis
        assert_eq!(orig.paragraphs.len(), deser.paragraphs.len());
        for (i, (o, d)) in orig.paragraphs.iter().zip(&deser.paragraphs).enumerate() {
            assert_eq!(
                o.sentences_ordered.len(),
                d.sentences_ordered.len(),
                "paragraphs[{i}].sentences_ordered.len"
            );
            for (j, (os, ds)) in o
                .sentences_ordered
                .iter()
                .zip(&d.sentences_ordered)
                .enumerate()
            {
                assert_eq!(os.0, ds.0, "paragraphs[{i}].sentences_ordered[{j}]");
            }
        }

        // Sentences analysis
        assert_eq!(orig.sentences.len(), deser.sentences.len());
        for (i, (o, d)) in orig.sentences.iter().zip(&deser.sentences).enumerate() {
            assert_eq!(
                o.words_ordered.len(),
                d.words_ordered.len(),
                "sentences[{i}].words_ordered.len"
            );
            for (j, (ow, dw)) in o.words_ordered.iter().zip(&d.words_ordered).enumerate() {
                assert_eq!(ow.0, dw.0, "sentences[{i}].words_ordered[{j}]");
            }
        }

        // Word analyses
        assert_eq!(orig.word_analyses.len(), deser.word_analyses.len());
        for (i, (o, d)) in orig
            .word_analyses
            .iter()
            .zip(&deser.word_analyses)
            .enumerate()
        {
            assert_eq!(
                o.origin_revision.0, d.origin_revision.0,
                "word_analyses[{i}].origin_revision"
            );
            assert_eq!(
                o.latest_revision.0, d.latest_revision.0,
                "word_analyses[{i}].latest_revision"
            );
            assert_eq!(
                o.inbound.len(),
                d.inbound.len(),
                "word_analyses[{i}].inbound.len"
            );
            for (j, (oi, di)) in o.inbound.iter().zip(&d.inbound).enumerate() {
                assert_eq!(oi.0, di.0, "word_analyses[{i}].inbound[{j}]");
            }
            assert_eq!(
                o.outbound.len(),
                d.outbound.len(),
                "word_analyses[{i}].outbound.len"
            );
            for (j, (oo, do_)) in o.outbound.iter().zip(&d.outbound).enumerate() {
                assert_eq!(oo.0, do_.0, "word_analyses[{i}].outbound[{j}]");
            }
        }

        // Public pointer fields
        assert_eq!(orig.spam_ids, deser.spam_ids);
        assert_eq!(orig.current_revision.0, deser.current_revision.0);
        assert_eq!(orig.ordered_revisions.len(), deser.ordered_revisions.len());
        for (i, (o, d)) in orig
            .ordered_revisions
            .iter()
            .zip(&deser.ordered_revisions)
            .enumerate()
        {
            assert_eq!(o.0, d.0, "ordered_revisions[{i}]");
        }
        assert_eq!(orig.words.len(), deser.words.len());
        for (i, (o, d)) in orig.words.iter().zip(&deser.words).enumerate() {
            assert_eq!(o.0, d.0, "words[{i}]");
        }
        assert_eq!(orig.revisions_by_id.len(), deser.revisions_by_id.len());
        for (id, orig_ptr) in &orig.revisions_by_id {
            let deser_ptr = deser
                .revisions_by_id
                .get(id)
                .unwrap_or_else(|| panic!("revisions_by_id missing key {id}"));
            assert_eq!(orig_ptr.0, deser_ptr.0, "revisions_by_id[{id}]");
        }
    }

    // -----------------------------------------------------------------------
    // Roundtrip tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_roundtrip_minimal() {
        let pa = PageAnalysis::new((RevisionAnalysis::default(), RevisionImmutables::dummy()));
        let json = serde_json::to_string(&pa).expect("serialize");
        let deser: PageAnalysis = serde_json::from_str(&json).expect("deserialize");

        assert_eq!(deser.current_revision.0, 0);
        assert_eq!(deser.revision_immutables.len(), 1);
        assert_eq!(deser.revisions.len(), 1);
        assert!(deser.paragraphs.is_empty());
        assert!(deser.sentences.is_empty());
        assert!(deser.word_analyses.is_empty());
        assert!(deser.words.is_empty());
        assert!(deser.spam_ids.is_empty());
        assert!(deser.ordered_revisions.is_empty());
        assert!(deser.internals.paragraphs_ht.is_empty());
        assert!(deser.internals.sentences_ht.is_empty());
        assert!(deser.internals.revision_prev.is_none());
    }

    #[test]
    fn test_roundtrip_full_structure() {
        let pa = build_test_page_analysis();
        let json = serde_json::to_string(&pa).expect("serialize");
        let deser: PageAnalysis = serde_json::from_str(&json).expect("deserialize");
        assert_roundtrip_eq(&pa, &deser);
    }

    #[test]
    fn test_roundtrip_bincode() {
        let pa = build_test_page_analysis();
        let bytes = bincode::serialize(&pa).expect("serialize");
        let deser: PageAnalysis = bincode::deserialize(&bytes).expect("deserialize");
        assert_roundtrip_eq(&pa, &deser);
    }

    #[test]
    fn test_roundtrip_with_real_analysis() {
        let revisions = vec![
            make_revision(1, "Hello world. This is a test."),
            make_revision(
                2,
                "Hello world. This is a modified test. New sentence added.",
            ),
            make_revision(3, "Hello world. New sentence added."),
        ];
        let pa = PageAnalysis::analyse_page(&revisions).expect("analyse_page");
        let json = serde_json::to_string(&pa).expect("serialize");
        let deser: PageAnalysis = serde_json::from_str(&json).expect("deserialize");
        assert_roundtrip_eq(&pa, &deser);
    }

    #[test]
    fn test_roundtrip_preserves_revision_data() {
        use crate::dump_parser::Sha1Hash;

        let revision = Revision {
            id: 42,
            timestamp: chrono::DateTime::from_timestamp_nanos(1_700_000_000_000_000_000),
            contributor: Contributor {
                id: Some(7),
                username: CompactString::from("SpecificUser"),
            },
            text: Text::Normal("Test text content".to_string()),
            sha1: Some(Sha1Hash(*b"abcdefghijklmnopqrstuvwxyz12345")),
            comment: Some(CompactString::from("my edit comment")),
            minor: true,
        };

        let mut pa = PageAnalysis::new((RevisionAnalysis::default(), RevisionImmutables::dummy()));
        let rev_ptr = pa.new_revision(RevisionImmutables::from_revision(&revision));
        pa.revisions_by_id.insert(42, rev_ptr.clone());
        pa.ordered_revisions.push(rev_ptr.clone());
        pa.current_revision = rev_ptr;

        let json = serde_json::to_string(&pa).expect("serialize");
        let deser: PageAnalysis = serde_json::from_str(&json).expect("deserialize");

        let d = &deser.revision_immutables[1];
        assert_eq!(d.id, 42);
        assert_eq!(d.text_lowercase, "test text content");
    }

    // -----------------------------------------------------------------------
    // Behavioral tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_blake3_hash_recomputed() {
        let pa = build_test_page_analysis();
        let json = serde_json::to_string(&pa).expect("serialize");
        let deser: PageAnalysis = serde_json::from_str(&json).expect("deserialize");

        for (i, (orig, d)) in pa
            .paragraph_immutables
            .iter()
            .zip(&deser.paragraph_immutables)
            .enumerate()
        {
            let expected = blake3::hash(d.value.as_bytes());
            assert_eq!(d.hash_value, expected, "paragraph[{i}] hash recomputed");
            assert_eq!(
                orig.hash_value, d.hash_value,
                "paragraph[{i}] hash matches original"
            );
        }
        for (i, (orig, d)) in pa
            .sentence_immutables
            .iter()
            .zip(&deser.sentence_immutables)
            .enumerate()
        {
            let expected = blake3::hash(d.value.as_bytes());
            assert_eq!(d.hash_value, expected, "sentence[{i}] hash recomputed");
            assert_eq!(
                orig.hash_value, d.hash_value,
                "sentence[{i}] hash matches original"
            );
        }
    }

    #[test]
    fn test_transient_fields_reset() {
        let mut pa = build_test_page_analysis();

        // Set transient flags to non-default before serializing
        pa.paragraphs[0].matched_in_current = true;
        pa.sentences[0].matched_in_current = true;
        pa.word_analyses[0].matched_in_current = true;
        pa.word_analyses[1].matched_in_current = true;

        let json = serde_json::to_string(&pa).expect("serialize");
        let deser: PageAnalysis = serde_json::from_str(&json).expect("deserialize");

        for (i, p) in deser.paragraphs.iter().enumerate() {
            assert!(!p.matched_in_current, "paragraphs[{i}].matched_in_current");
            assert!(
                p.sentences_by_hash.is_empty(),
                "paragraphs[{i}].sentences_by_hash"
            );
        }
        for (i, s) in deser.sentences.iter().enumerate() {
            assert!(!s.matched_in_current, "sentences[{i}].matched_in_current");
        }
        for (i, w) in deser.word_analyses.iter().enumerate() {
            assert!(
                !w.matched_in_current,
                "word_analyses[{i}].matched_in_current"
            );
        }
        for (i, r) in deser.revisions.iter().enumerate() {
            assert!(
                r.paragraphs_by_hash.is_empty(),
                "revisions[{i}].paragraphs_by_hash"
            );
        }
        assert!(deser.internals.paragraphs_ht.is_empty());
        assert!(deser.internals.sentences_ht.is_empty());
        assert!(deser.internals.revision_prev.is_none());
    }

    #[test]
    fn test_arc_sharing() {
        let pa = build_test_page_analysis();
        let json = serde_json::to_string(&pa).expect("serialize");
        let deser: PageAnalysis = serde_json::from_str(&json).expect("deserialize");

        // Both words share origin_revision index 1
        assert_eq!(deser.word_analyses[0].origin_revision.0, 1);
        assert_eq!(deser.word_analyses[1].origin_revision.0, 1);
        assert!(
            Arc::ptr_eq(
                &deser.word_analyses[0].origin_revision.1,
                &deser.word_analyses[1].origin_revision.1
            ),
            "shared origin_revision should be the same Arc allocation"
        );

        // ordered_revisions and revisions_by_id share Arcs for same index
        let ordered_rev1 = deser
            .ordered_revisions
            .iter()
            .find(|r| r.0 == 1)
            .expect("rev index 1 in ordered_revisions");
        let byid_rev1 = deser
            .revisions_by_id
            .get(&1)
            .expect("rev id 1 in revisions_by_id");
        assert!(
            Arc::ptr_eq(&ordered_rev1.1, &byid_rev1.1),
            "same revision index should share Arc across ordered_revisions and revisions_by_id"
        );
    }

    // -----------------------------------------------------------------------
    // Error handling tests
    // -----------------------------------------------------------------------

    fn assert_deser_error(val: serde_json::Value, expected_fragments: &[&str]) {
        match serde_json::from_value::<PageAnalysis>(val) {
            Ok(_) => panic!("deserialization should have failed"),
            Err(err) => {
                let msg = err.to_string();
                for frag in expected_fragments {
                    assert!(msg.contains(frag), "error should contain {frag:?}: {msg}");
                }
            }
        }
    }

    #[test]
    fn test_error_revision_index_out_of_bounds() {
        let pa = PageAnalysis::new((RevisionAnalysis::default(), RevisionImmutables::dummy()));
        let mut val: serde_json::Value = serde_json::to_value(&pa).expect("serialize");
        val["current_revision"] = serde_json::json!(999);
        assert_deser_error(val, &["revision index", "out of bounds"]);
    }

    #[test]
    fn test_error_paragraph_index_out_of_bounds() {
        let pa = build_test_page_analysis();
        let mut val: serde_json::Value = serde_json::to_value(&pa).expect("serialize");
        val["revisions"][1]["paragraphs_ordered"][0] = serde_json::json!(999);
        assert_deser_error(val, &["paragraph index", "out of bounds"]);
    }

    #[test]
    fn test_error_sentence_index_out_of_bounds() {
        let pa = build_test_page_analysis();
        let mut val: serde_json::Value = serde_json::to_value(&pa).expect("serialize");
        val["paragraphs"][0]["sentences_ordered"][0] = serde_json::json!(999);
        assert_deser_error(val, &["sentence index", "out of bounds"]);
    }

    #[test]
    fn test_error_word_index_out_of_bounds() {
        let pa = build_test_page_analysis();
        let mut val: serde_json::Value = serde_json::to_value(&pa).expect("serialize");
        val["sentences"][0]["words_ordered"][0] = serde_json::json!(999);
        assert_deser_error(val, &["word index", "out of bounds"]);
    }
}
