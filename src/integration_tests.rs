use std::collections::HashMap;

use pyo3::types::PyDict;

use crate::{
    algorithm::{Analysis, AnalysisError},
    dump_parser::{Page, Revision, Text},
    test_support::{prelude::*, PyParagraph, PySentence, PyWikiwho},
};

fn run_analysis_python(py: Python<'_>, page: &Page) -> PyWikiwho {
    let page_xml = page_to_xml(page);
    let locals = PyDict::new_bound(py);
    locals.set_item("input", page_xml).unwrap();

    py.run_bound(
        "
from mwxml import Dump
from mwtypes.files import reader

#import WikiWho
from WikiWho.wikiwho import Wikiwho
from WikiWho.utils import split_into_paragraphs

# more info about reading xml dumps: https://github.com/mediawiki-utilities/python-mwxml
dump = Dump.from_page_xml(input)
for page in dump:
wikiwho = Wikiwho(page.title)
revisions = list(page)
wikiwho.analyse_article_from_xml_dump(revisions)
#print(WikiWho.__file__)
#print(split_into_paragraphs(revisions[0].text.lower()))
break  # process only first page
",
        None,
        Some(&locals),
    )
    .unwrap();

    locals
        .get_item("wikiwho")
        .unwrap()
        .unwrap()
        .extract::<PyWikiwho>()
        .unwrap()
}

#[test]
fn test_case_1() {
    Python::with_gil(|py| {
        let page = Page {
            title: "Test".into(),
            namespace: 0,
            revisions: vec![
                Revision {
                    id: 1,
                    text: Text::Deleted,
                    ..dummy_revision()
                },
                Revision {
                    id: 2,
                    text: Text::Normal("®\u{2000}￼".into()),
                    ..dummy_revision()
                },
            ],
        };

        let (analysis, analysis_result) = Analysis::analyse_page(&page.revisions).unwrap();
        let wikiwho_py = run_analysis_python(py, &page);

        let sentence_rust = {
            let paragraph = &analysis[&analysis_result.revisions[&2]].paragraphs_ordered[0];
            let sentence_pointer = &analysis[paragraph].sentences_ordered[0];
            &analysis[sentence_pointer]
        };
        let sentence_py = {
            let revision = &wikiwho_py.revisions[&2];
            let paragraph_hash = &revision.ordered_paragraphs[0];
            let paragraph = &revision.paragraphs[paragraph_hash][0];
            let sentence_hash = &paragraph.ordered_sentences[0];
            &paragraph.sentences[sentence_hash][0]
        };

        assert_eq!(sentence_rust.words_ordered.len(), sentence_py.words.len());

        for (word_rust, word_py) in sentence_rust
            .words_ordered
            .iter()
            .zip(sentence_py.words.iter())
        {
            assert_eq!(word_rust.value, word_py.value);
        }
    })
}

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 1000,
        max_shrink_iters: 40000,
        ..ProptestConfig::default()
    })]
    #[test]
    fn compare_algorithm_python(page in proptest_support::correct_page()) {
        with_gil!(py, {
            // run Rust implementation
            let result = Analysis::analyse_page(&page.revisions);
            // reject test case if there are no valid revisions
            prop_assume!(!matches!(result, Err(AnalysisError::NoValidRevisions)));
            let (analysis, analysis_result) = result.unwrap();

            // run Python implementation
            let wikiwho_py = run_analysis_python(py, &page);

            // iterate and compare result graph
            for revision_id in page.revisions.iter().map(|r| r.id) {
                // check spam
                let is_spam_rust = analysis_result.spam_ids.contains(&revision_id);
                let is_spam_py = wikiwho_py.spam_ids.contains(&revision_id);
                prop_assert_eq!(is_spam_rust, is_spam_py);

                if is_spam_rust {
                    // spam revisions are not analysed further
                    continue;
                }
                let input_revision =
                    page.revisions.iter().find(|r| r.id == revision_id).unwrap();
                if input_revision.text.len() == 0 {
                    // empty revisions are not analysed further
                    continue;
                }

                // compare revisions

                let revision_pointer_rust = &analysis_result.revisions[&revision_id];
                let revision_py = wikiwho_py.revisions.get(&revision_id).unwrap();

                prop_assert_eq!(revision_pointer_rust.id, revision_py.id);

                let revision_rust = &analysis[revision_pointer_rust];
                let paragraphs_py = &revision_py.ordered_paragraphs;
                prop_assert_eq!(revision_rust.paragraphs_ordered.len(), paragraphs_py.len());

                let mut paragraph_hash_disambiguation = HashMap::new();
                for (paragraph_pointer_rust, paragraph_hash_py) in revision_rust
                    .paragraphs_ordered
                    .iter()
                    .zip(paragraphs_py.iter())
                {
                    // compare paragraphs

                    let count: usize = *paragraph_hash_disambiguation
                        .entry(paragraph_hash_py)
                        .and_modify(|count| {
                            *count += 1;
                        })
                        .or_default();
                    let paragraph_py: &PyParagraph =
                        &revision_py.paragraphs.get(paragraph_hash_py).unwrap()[count];
                    prop_assert_eq!(&paragraph_pointer_rust.value, &paragraph_py.value);

                    let paragraph_rust = &analysis[paragraph_pointer_rust];
                    let sentences_py = &paragraph_py.ordered_sentences;
                    prop_assert_eq!(paragraph_rust.sentences_ordered.len(), sentences_py.len());

                    let mut sentence_hash_disambiguation = HashMap::new();
                    for (sentence_pointer_rust, sentence_hash_py) in paragraph_rust
                        .sentences_ordered
                        .iter()
                        .zip(sentences_py.iter())
                    {
                        // compare sentences

                        let count: usize = *sentence_hash_disambiguation
                            .entry(sentence_hash_py)
                            .and_modify(|count| {
                                *count += 1;
                            })
                            .or_default();
                        let sentence_py: &PySentence =
                            &paragraph_py.sentences.get(sentence_hash_py).unwrap()[count];
                        prop_assert_eq!(&sentence_pointer_rust.value, &sentence_py.value);

                        let sentence_rust = &analysis[sentence_pointer_rust];
                        let words_py = &sentence_py.words;
                        prop_assert_eq!(sentence_rust.words_ordered.len(), words_py.len());

                        for (word_pointer_rust, word_py) in
                            sentence_rust.words_ordered.iter().zip(words_py.iter())
                        {
                            // compare words

                            prop_assert_eq!(&word_pointer_rust.value, &word_py.value);
                        }
                    }
                }
            }
        })
    }
}
