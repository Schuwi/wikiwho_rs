// SPDX-License-Identifier: MPL-2.0
use std::{collections::HashMap, fs::File, io::BufReader};

use pyo3::types::PyDict;

use crate::{
    algorithm::{Analysis, AnalysisError},
    dump_parser::{DumpParser, Page, Revision, Text},
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

        let analysis = Analysis::analyse_page(&page.revisions).unwrap();
        let wikiwho_py = run_analysis_python(py, &page);

        let sentence_rust = {
            let paragraph = &analysis[&analysis.revisions_by_id[&2]].paragraphs_ordered[0];
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

#[test]
fn test_case_2() {
    // found by proptest
    let page = Page {
        title: "Test".into(),
        namespace: 0,
        revisions: vec![
            Revision {
                id: 1,
                text: Text::Normal("funny.-.".into()),
                ..dummy_revision()
            },
            Revision {
                id: 2,
                text: Text::Normal("-.some".into()),
                ..dummy_revision()
            },
        ],
    };

    compare_algorithm_python(&page).unwrap();
}

fn compare_algorithm_python(page: &Page) -> Result<(), TestCaseError> {
    with_gil!(py, {
        // run Rust implementation
        let result = Analysis::analyse_page(&page.revisions);
        // reject test case if there are no valid revisions
        prop_assume!(!matches!(result, Err(AnalysisError::NoValidRevisions)));
        let analysis = result.unwrap();

        // run Python implementation
        let wikiwho_py = run_analysis_python(py, &page);
        prop_assert_eq!(
            wikiwho_py.ordered_revisions.last(),
            Some(&wikiwho_py.revision_curr.id)
        );

        prop_assert_eq!(
            &analysis
                .ordered_revisions
                .iter()
                .map(|i| i.id)
                .collect::<Vec<_>>(),
            &wikiwho_py.ordered_revisions
        );

        // iterate and compare result graph
        for revision_id in page.revisions.iter().map(|r| r.id) {
            // check spam
            let is_spam_rust = analysis.spam_ids.contains(&revision_id);
            let is_spam_py = wikiwho_py.spam_ids.contains(&revision_id);
            prop_assert_eq!(is_spam_rust, is_spam_py);

            if is_spam_rust {
                // spam revisions are not analysed further
                continue;
            }
            let input_revision = page.revisions.iter().find(|r| r.id == revision_id).unwrap();
            if input_revision.text.len() == 0 {
                // empty revisions are not analysed further
                continue;
            }

            // compare revisions

            let revision_pointer_rust = &analysis.revisions_by_id[&revision_id];
            let revision_py = wikiwho_py.revisions.get(&revision_id).unwrap();

            prop_assert_eq!(revision_pointer_rust.id, revision_py.id);

            let revision_rust = &analysis[revision_pointer_rust];
            let paragraphs_py = &revision_py.ordered_paragraphs;
            prop_assert_eq!(revision_rust.paragraphs_ordered.len(), paragraphs_py.len());
            prop_assert_eq!(revision_rust.original_adds, revision_py.original_adds);

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

                        let word_rust = &analysis[word_pointer_rust];
                        prop_assert_eq!(word_pointer_rust.unique_id(), word_py.token_id as usize);
                        prop_assert_eq!(
                            &word_rust.inbound.iter().map(|i| i.id).collect::<Vec<_>>(),
                            &word_py.inbound
                        );
                        prop_assert_eq!(
                            &word_rust.outbound.iter().map(|i| i.id).collect::<Vec<_>>(),
                            &word_py.outbound
                        );
                        prop_assert_eq!(
                            word_rust.latest_revision.id,
                            word_py.last_rev_id,
                            "inconsistency at word: {:?}, revision: {}",
                            &word_pointer_rust.value,
                            revision_id
                        );
                        prop_assert_eq!(word_rust.origin_revision.id, word_py.origin_rev_id);
                    }
                }
            }
        }
    });
    Ok(())
}

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 1000,
        max_shrink_iters: 40000,
        ..ProptestConfig::default()
    })]
    #[test]
    fn random_unicode_page(page in proptest_support::correct_page(r"\PC*".boxed(), 50)) {
        // \0 character fails XML parsing in python
        if let Err(err) = compare_algorithm_python(&page) {
            // don't ask, the proptest macro is a bit weird
            return Err(err);
        }
    }
}

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 10000,
        max_shrink_iters: 40000,
        ..ProptestConfig::default()
    })]
    #[test]
    fn tokenized_page(page in proptest_support::correct_page("(some|funny|words|\\.|\\{\\{|\\}\\}|\\PC| |-|\n|&|;|'|\\]|\\[|\\||no|yes|why)*".boxed(), 10)) {
        // \0 character fails XML parsing in python
        if let Err(err) = compare_algorithm_python(&page) {
            // don't ask, the proptest macro is a bit weird
            return Err(err);
        }
    }
}

#[test]
fn known_bad_example_familia() {
    let reader = BufReader::new(File::open("failing-inputs/familia.xml").unwrap());
    let mut parser = DumpParser::new(reader).unwrap();
    let page = parser.parse_page().unwrap().unwrap();

    compare_algorithm_python(&page).unwrap();
}

// static SITE_INFO: LazyLock<SiteInfo> = LazyLock::new(|| {
//     let mut namespaces = HashMap::new();

//     /*
//     <namespace key="-2" case="case-sensitive">Medium</namespace>
//       <namespace key="-1" case="first-letter">Spezial</namespace>
//       <namespace key="0" case="case-sensitive" />
//       <namespace key="1" case="case-sensitive">Diskussion</namespace>
//       <namespace key="2" case="first-letter">Benutzer</namespace>
//       <namespace key="3" case="first-letter">Benutzer Diskussion</namespace>
//       <namespace key="4" case="case-sensitive">Wiktionary</namespace>
//       <namespace key="5" case="case-sensitive">Wiktionary Diskussion</namespace>
//       <namespace key="6" case="case-sensitive">Datei</namespace>
//       <namespace key="7" case="case-sensitive">Datei Diskussion</namespace>
//       <namespace key="8" case="first-letter">MediaWiki</namespace>
//       <namespace key="9" case="first-letter">MediaWiki Diskussion</namespace>
//       <namespace key="10" case="case-sensitive">Vorlage</namespace>
//       <namespace key="11" case="case-sensitive">Vorlage Diskussion</namespace>
//       <namespace key="12" case="case-sensitive">Hilfe</namespace>
//       <namespace key="13" case="case-sensitive">Hilfe Diskussion</namespace>
//       <namespace key="14" case="case-sensitive">Kategorie</namespace>
//       <namespace key="15" case="case-sensitive">Kategorie Diskussion</namespace>
//       <namespace key="102" case="case-sensitive">Verzeichnis</namespace>
//       <namespace key="103" case="case-sensitive">Verzeichnis Diskussion</namespace>
//       <namespace key="104" case="case-sensitive">Thesaurus</namespace>
//       <namespace key="105" case="case-sensitive">Thesaurus Diskussion</namespace>
//       <namespace key="106" case="case-sensitive">Reim</namespace>
//       <namespace key="107" case="case-sensitive">Reim Diskussion</namespace>
//       <namespace key="108" case="case-sensitive">Flexion</namespace>
//       <namespace key="109" case="case-sensitive">Flexion Diskussion</namespace>
//       <namespace key="110" case="case-sensitive">Rekonstruktion</namespace>
//       <namespace key="111" case="case-sensitive">Rekonstruktion Diskussion</namespace>
//       <namespace key="710" case="case-sensitive">TimedText</namespace>
//       <namespace key="711" case="case-sensitive">TimedText talk</namespace>
//       <namespace key="828" case="case-sensitive">Modul</namespace>
//       <namespace key="829" case="case-sensitive">Modul Diskussion</namespace>
//      */
//     namespaces.insert(-2, Namespace::Named("Medium".into()));
//     namespaces.insert(-1, Namespace::Named("Spezial".into()));
//     namespaces.insert(0, Namespace::Default);
//     namespaces.insert(1, Namespace::Named("Diskussion".into()));
//     namespaces.insert(2, Namespace::Named("Benutzer".into()));
//     namespaces.insert(3, Namespace::Named("Benutzer Diskussion".into()));
//     namespaces.insert(4, Namespace::Named("Wiktionary".into()));
//     namespaces.insert(5, Namespace::Named("Wiktionary Diskussion".into()));
//     namespaces.insert(6, Namespace::Named("Datei".into()));
//     namespaces.insert(7, Namespace::Named("Datei Diskussion".into()));
//     namespaces.insert(8, Namespace::Named("MediaWiki".into()));
//     namespaces.insert(9, Namespace::Named("MediaWiki Diskussion".into()));
//     namespaces.insert(10, Namespace::Named("Vorlage".into()));
//     namespaces.insert(11, Namespace::Named("Vorlage Diskussion".into()));
//     namespaces.insert(12, Namespace::Named("Hilfe".into()));
//     namespaces.insert(13, Namespace::Named("Hilfe Diskussion".into()));
//     namespaces.insert(14, Namespace::Named("Kategorie".into()));
//     namespaces.insert(15, Namespace::Named("Kategorie Diskussion".into()));
//     namespaces.insert(102, Namespace::Named("Verzeichnis".into()));
//     namespaces.insert(103, Namespace::Named("Verzeichnis Diskussion".into()));
//     namespaces.insert(104, Namespace::Named("Thesaurus".into()));
//     namespaces.insert(105, Namespace::Named("Thesaurus Diskussion".into()));
//     namespaces.insert(106, Namespace::Named("Reim".into()));
//     namespaces.insert(107, Namespace::Named("Reim Diskussion".into()));
//     namespaces.insert(108, Namespace::Named("Flexion".into()));
//     namespaces.insert(109, Namespace::Named("Flexion Diskussion".into()));
//     namespaces.insert(110, Namespace::Named("Rekonstruktion".into()));
//     namespaces.insert(111, Namespace::Named("Rekonstruktion Diskussion".into()));
//     namespaces.insert(710, Namespace::Named("TimedText".into()));
//     namespaces.insert(711, Namespace::Named("TimedText talk".into()));
//     namespaces.insert(828, Namespace::Named("Modul".into()));
//     namespaces.insert(829, Namespace::Named("Modul Diskussion".into()));

//     SiteInfo {
//         dbname: CompactString::const_new("dewiktionary"),
//         namespaces,
//     }
// });

#[test]
fn known_bad_example_anontalkpagetext() {
    let reader =
        BufReader::new(File::open("failing-inputs/Anontalkpagetext_shortened.xml").unwrap());
    let mut parser = DumpParser::new(reader).unwrap();
    let page = parser.parse_page().unwrap().unwrap();

    compare_algorithm_python(&page).unwrap();
}

// delta debugging
use crate::test_support::delta_debug_texts;

#[test]
#[ignore] // this test takes very long and is only useful for focus debugging
fn simplify_bad_example_anontalkpagetext() {
    let reader = BufReader::new(
        File::open("failing-inputs/Anontalkpagetext_shortened-manually.xml").unwrap(),
    );
    let mut parser = DumpParser::new(reader).unwrap();
    let bad_page = parser.parse_page().unwrap().unwrap();

    let test_page =
        |page: &Page| matches!(compare_algorithm_python(page), Err(TestCaseError::Fail(_)));

    // Ensure the bad_page indeed causes a failure
    assert!(
        test_page(&bad_page),
        "The provided bad_page does not cause a failure."
    );

    // Perform delta debugging on texts
    let minimized_page = delta_debug_texts(
        bad_page, test_page, 300000, /* runs for about an hour or so */
    );

    // Assert that the minimized_page still causes the failure
    assert!(
        test_page(&minimized_page),
        "The minimized_page does not cause a failure."
    );

    // Debug some weird inconsistency that pasting the minimized page into the XML file suddenly no longer causes a failure
    // {
    //     let reader =
    //     BufReader::new(File::open("failing-inputs/Anontalkpagetext_shortened.xml").unwrap());
    //     let mut parser = DumpParser::new(reader).unwrap();
    //     let compare_page = parser.parse_page().unwrap().unwrap();

    //     assert_eq!(minimized_page, compare_page);
    // }
    // Conclusion: Make sure VS Code does NOT add indentations when pasting the minimized page into the XML file!!

    // Output the minimized Page for inspection
    println!("\n\n\n\nMinimized Page: {}", page_to_xml(&minimized_page));
}

#[test]
fn known_bad_example_hallo() {
    let reader = BufReader::new(File::open("failing-inputs/Hallo.xml").unwrap());
    let mut parser = DumpParser::new(reader).unwrap();
    let page = parser.parse_page().unwrap().unwrap();

    compare_algorithm_python(&page).unwrap();

    println!("Page: {}", page_to_xml(&page));
}
