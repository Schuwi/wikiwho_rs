// SPDX-License-Identifier: MPL-2.0
// it only makes sense to compare the algorithm to python if the same diff algorithm is used
#![cfg(feature = "python-diff")]

use std::{
    collections::HashMap,
    fs::File,
    io::{BufReader, Read, Seek, SeekFrom, Write},
    path::PathBuf,
    sync::mpsc::{Receiver, Sender},
};

use pyo3::{import_exception, types::PyDict};

use wikiwho::{
    algorithm::{Analysis, AnalysisError},
    dump_parser::{DumpParser, Page, Revision, Text},
};

mod common;

use common::{input_structs, output_structs};
use common::{load_local_module, prelude::*};

#[derive(Clone, Copy)]
struct PageRef {
    offset: u64,
    length: u64,
}

fn run_analysis_python(py: Python<'_>, page: &Page) -> output_structs::PyWikiwho {
    let page_py = input_structs::PyPage::from_page(page);
    let locals = PyDict::new_bound(py);
    locals.set_item("page", page_py.into_py(py)).unwrap();

    py.run_bound(
        "
from WikiWho.wikiwho import Wikiwho

wikiwho = Wikiwho('') # title is not relevant for algorithm behavior
wikiwho.analyse_article_from_xml_dump(page)
",
        None,
        Some(&locals),
    )
    .unwrap();

    locals
        .get_item("wikiwho")
        .unwrap()
        .unwrap()
        .extract::<output_structs::PyWikiwho>()
        .unwrap()
}

///
///
/// Will run the analysis in a separate python process using multiprocessing.Pool.
/// Thus it will not block the main thread.
///
/// # Returns
///
/// The `Process` object that is running the analysis.
fn run_analysis_python_mt(
    py: Python<'_>,
    work_receiver: Receiver<PageRef>,
    result_sender: Sender<output_structs::PyWikiwho>,
    temp_path: PathBuf,
) -> Bound<'_, PyAny> {
    let threads = std::thread::available_parallelism().unwrap().get() - 1;
    let threads = usize::max(1, threads);

    let py_support = load_local_module(py, "tests.support").unwrap();

    // Register pyo3 input types with tests.support so pickle can find them by module path.
    // This must happen before pool.imap_unordered, which forks workers that inherit sys.modules.
    py_support.add_class::<input_structs::PyPage>().unwrap();
    py_support.add_class::<input_structs::PyRevision>().unwrap();
    py_support.add_class::<input_structs::PyDeleted>().unwrap();
    py_support
        .add_class::<input_structs::PyTimestamp>()
        .unwrap();

    let result = py_support
        .getattr("run_analysis_python_mt")
        .unwrap()
        .call1((threads,))
        .unwrap()
        .downcast_into::<PyDict>()
        .unwrap();

    let py_work_receiver = result.get_item("work_receiver").unwrap().unwrap().unbind();
    let py_result_sender = result.get_item("result_sender").unwrap().unwrap().unbind();

    // Bridge thread: read pages from temp file and send to Python pool
    std::thread::spawn(move || {
        let mut file = File::open(&temp_path).unwrap();
        let mut buf = Vec::new();
        while let Ok(page_ref) = work_receiver.recv() {
            file.seek(SeekFrom::Start(page_ref.offset)).unwrap();
            buf.resize(page_ref.length as usize, 0);
            file.read_exact(&mut buf).unwrap();
            let page: Page = bincode::deserialize(&buf).unwrap();
            Python::with_gil(|py| {
                let page_py = input_structs::PyPage::from_page(&page);
                py_work_receiver
                    .call_method1(py, "put_nowait", (page_py,))
                    .unwrap();
            });
        }
        // signal end of work to Python pool (support.py uses iter(queue.get, None))
        Python::with_gil(|py| {
            py_work_receiver
                .call_method1(py, "put_nowait", (py.None(),))
                .unwrap();
        });
    });

    std::thread::spawn(move || {
        import_exception!(queue, Empty);
        let mut received = 0;
        loop {
            // Acquire the GIL only for one get() call at a time.
            // Queue.get(timeout) releases the GIL internally while waiting,
            // allowing the bridge thread to enqueue pages concurrently.
            let item =
                Python::with_gil(
                    |py| match py_result_sender.call_method1(py, "get", (0.5f64,)) {
                        Ok(obj) => {
                            if obj.extract::<String>(py).ok().as_deref() == Some("close") {
                                return Ok(None);
                            }
                            let page: output_structs::PyWikiwho = obj.extract(py).unwrap();
                            received += 1;
                            Ok(Some(page))
                        }
                        Err(err) if err.is_instance_of::<Empty>(py) => Err(()),
                        Err(err) => panic!("Error in python process: {:?}", err),
                    },
                );
            match item {
                Ok(Some(page)) => result_sender.send(page).unwrap(),
                Ok(None) => break,
                Err(()) => {} // timeout, retry
            }
        }
        println!("Python processing done, received {received} results");
    });

    result.get_item("process").unwrap().unwrap()
}

#[test]
fn test_case_1() {
    // found by proptest
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

fn compare_results(
    page: &Page,
    analysis: &Analysis,
    wikiwho_py: &output_structs::PyWikiwho,
) -> Result<(), TestCaseError> {
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
            let paragraph_py: &output_structs::PyParagraph =
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
                let sentence_py: &output_structs::PySentence =
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
    Ok(())
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
        compare_results(page, &analysis, &wikiwho_py)?;
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
    let reader = common::open_test_dump();
    let page: Page = common::find_page_by_title_and_ns(reader, "familia", 0)
        .unwrap()
        .unwrap();

    compare_algorithm_python(&page).unwrap();
}

#[test]
fn known_bad_example_anontalkpagetext() {
    let page: Page = serde_json::from_reader(
        File::open("failing-inputs/Anontalkpagetext_shortened.json").unwrap(),
    )
    .unwrap();

    compare_algorithm_python(&page).unwrap();
}

// delta debugging
use common::delta_debug_texts;

#[test]
#[ignore] // this "test" takes very long and is only useful for focus debugging
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

    // Output the minimized Page for inspection
    println!(
        "\n\n\n\nMinimized Page: \n{}",
        serde_json::to_string_pretty(&minimized_page).unwrap()
    );
}

#[test]
fn known_bad_example_hallo() {
    let reader = common::open_test_dump();
    let page: Page = common::find_page_by_title_and_ns(reader, "Hallo", 0)
        .unwrap()
        .unwrap();

    compare_algorithm_python(&page).unwrap();
}

#[test]
#[ignore] // takes quite some time to skim the whole dump twice
fn random_pages_100() {
    let reader1 = common::open_test_dump();
    let reader2 = common::open_test_dump();
    let pages = common::pick_n_random_pages((reader1, reader2), 100, 0).unwrap();

    for page in pages {
        compare_algorithm_python(&page).unwrap();
    }
}

#[test]
fn first_1000_pages_mt() {
    const PAGE_COUNT: usize = 200; // TODO: fix some memory issues and increase to 1000 or more

    let reader = common::open_test_dump();
    let temp_path = std::env::temp_dir().join(format!("wikiwho_test_{}.bin", std::process::id()));
    let cleanup_path = temp_path.clone();

    // pre-create temp file so consumer threads can open it before the parser starts writing
    File::create(&temp_path).unwrap();

    // python process
    let (py_sender, py_receiver) = {
        let (work_sender, work_receiver) = std::sync::mpsc::channel::<PageRef>();
        let (result_sender, result_receiver) = std::sync::mpsc::channel();

        Python::with_gil(|py| {
            run_analysis_python_mt(py, work_receiver, result_sender, temp_path.clone());
        });

        (work_sender, result_receiver)
    };

    // rust thread
    let (rust_sender, rust_receiver) = {
        let (work_sender, work_receiver) = std::sync::mpsc::channel::<PageRef>();
        let (result_sender, result_receiver) = std::sync::mpsc::channel();
        let rust_temp_path = temp_path.clone();

        std::thread::spawn(move || {
            let mut file = File::open(&rust_temp_path).unwrap();
            let mut buf = Vec::new();

            let mut processed = 0;

            for page_ref in work_receiver {
                file.seek(SeekFrom::Start(page_ref.offset)).unwrap();
                buf.resize(page_ref.length as usize, 0);
                file.read_exact(&mut buf).unwrap();
                let page: Page = bincode::deserialize(&buf).unwrap();
                let key = format!("{}:{}", page.namespace, page.title);
                let analysis = Analysis::analyse_page(&page.revisions).unwrap();
                result_sender.send((key, page_ref, analysis)).unwrap();

                processed += 1;
            }

            println!("Rust thread done, processed {processed} pages");
        });

        (work_sender, result_receiver)
    };

    // parser/producer thread — serializes pages to temp file, sends PageRefs via unbounded channels
    std::thread::spawn(move || {
        let mut file = File::create(&temp_path).unwrap();
        let mut parser = DumpParser::new(BufReader::new(reader)).unwrap();
        let mut offset: u64 = 0;
        for _ in 0..PAGE_COUNT {
            let page = parser.parse_page().unwrap().unwrap();
            let bytes = bincode::serialize(&page).unwrap();
            let length = bytes.len() as u64;
            file.write_all(&bytes).unwrap();
            let page_ref = PageRef { offset, length };
            py_sender.send(page_ref).unwrap();
            rust_sender.send(page_ref).unwrap();
            offset += length;
        }

        println!("Producer thread done, wrote {PAGE_COUNT} pages to temp file");
    });

    // Main matching loop — polls both result channels, compares when both sides are ready.
    // Pages are re-read from the temp file on demand to avoid holding them in memory.
    enum PendingResult {
        RustDone {
            page_ref: PageRef,
            analysis: Analysis,
        },
        PyDone {
            analysis_py: output_structs::PyWikiwho,
        },
    }

    let mut main_file = File::open(&cleanup_path).unwrap();
    let mut main_buf = Vec::new();
    let read_page = |file: &mut File, buf: &mut Vec<u8>, page_ref: PageRef| -> Page {
        file.seek(SeekFrom::Start(page_ref.offset)).unwrap();
        buf.resize(page_ref.length as usize, 0);
        file.read_exact(buf).unwrap();
        bincode::deserialize(buf).unwrap()
    };

    let mut pending: HashMap<String, PendingResult> = HashMap::new();
    let mut rust_done = false;
    let mut py_done = false;
    let mut compared = 0;

    loop {
        if !rust_done {
            match rust_receiver.try_recv() {
                Ok((key, page_ref, analysis)) => {
                    if let Some(PendingResult::PyDone { analysis_py }) = pending.remove(&key) {
                        let page = read_page(&mut main_file, &mut main_buf, page_ref);
                        compare_results(&page, &analysis, &analysis_py).unwrap();
                        compared += 1;
                    } else {
                        pending.insert(key, PendingResult::RustDone { page_ref, analysis });
                    }
                }
                Err(std::sync::mpsc::TryRecvError::Disconnected) => rust_done = true,
                _ => {}
            }
        }
        if !py_done {
            match py_receiver.try_recv() {
                Ok(analysis_py) => {
                    let key = analysis_py.title.clone();
                    if let Some(PendingResult::RustDone { page_ref, analysis }) =
                        pending.remove(&key)
                    {
                        let page = read_page(&mut main_file, &mut main_buf, page_ref);
                        compare_results(&page, &analysis, &analysis_py).unwrap();
                        compared += 1;
                    } else {
                        pending.insert(key, PendingResult::PyDone { analysis_py });
                    }
                }
                Err(std::sync::mpsc::TryRecvError::Disconnected) => py_done = true,
                _ => {}
            }
        }

        if rust_done && py_done {
            println!("All results received. Compared {compared} pages.");
            break;
        }

        std::thread::sleep(std::time::Duration::from_millis(20));
    }

    assert!(
        pending.is_empty(),
        "unmatched results: {:?}",
        pending.keys().collect::<Vec<_>>()
    );
    let _ = std::fs::remove_file(&cleanup_path);
}
