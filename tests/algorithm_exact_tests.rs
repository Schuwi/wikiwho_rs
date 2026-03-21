// SPDX-License-Identifier: MPL-2.0
// it only makes sense to compare the algorithm to python if the same diff algorithm is used
#![cfg(feature = "python-diff")]

use std::{
    collections::HashMap,
    fs::File,
    io::{BufReader, Read, Seek, SeekFrom, Write},
    sync::mpsc::Sender,
};

use pyo3::{import_exception, types::PyDict};

use wikiwho::{
    algorithm::{AnalysisError, PageAnalysis},
    dump_parser::{DumpParser, Page, Revision, Text},
};

mod common;

use common::input_structs;
use common::output_structs::serialize_wikiwho_result;
use common::{load_local_module, prelude::*};

#[derive(Clone, Copy)]
struct PageRef {
    offset: u64,
    length: u64,
}

fn run_analysis_python(py: Python<'_>, page: &Page) -> PageAnalysis {
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

    let py_wikiwho = locals.get_item("wikiwho").unwrap().unwrap();

    page_analysis_from_wikiwho(&py_wikiwho, page).unwrap()
}

/// Starts a Python multiprocessing.Pool in a separate Process and spawns a result
/// collection thread. Python workers read page bincode directly from `input_path`
/// and write result bincode to per-worker files in `result_dir`.
///
/// Returns the Python work queue (`Py<PyAny>`). The caller puts `(offset, length)`
/// tuples into it and sends `None` to signal completion.
fn run_analysis_python_mt(
    py: Python<'_>,
    result_sender: Sender<(String, PageAnalysis)>,
    input_path: &std::path::Path,
    result_dir: &std::path::Path,
) -> Py<PyAny> {
    // leave some headroom for the Rust side
    let threads = std::thread::available_parallelism().unwrap().get() - 2;
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

    py_support
        .add_function(wrap_pyfunction!(serialize_wikiwho_result, &py_support).unwrap())
        .unwrap();

    let result = py_support
        .getattr("run_analysis_python_mt")
        .unwrap()
        .call1((
            threads,
            input_path.to_str().unwrap(),
            result_dir.to_str().unwrap(),
        ))
        .unwrap()
        .downcast_into::<PyDict>()
        .unwrap();

    let py_work_queue = result.get_item("work_receiver").unwrap().unwrap().unbind();
    let py_result_queue = result.get_item("result_sender").unwrap().unwrap().unbind();

    // Result collection thread: receive (key, path, offset, length) tuples from Python,
    // read result bincode from per-worker files outside the GIL, and forward to main thread.
    std::thread::spawn(move || {
        import_exception!(queue, Empty);

        let mut last_log_time = std::time::Instant::now();
        let mut received = 0;

        let mut file_cache: HashMap<String, File> = HashMap::new();
        let mut result_buffer = Vec::new();
        loop {
            // Acquire the GIL only to extract a small metadata tuple.
            // Queue.get(timeout) releases the GIL internally while waiting.
            let item =
                Python::with_gil(
                    |py| match py_result_queue.call_method1(py, "get", (0.5f64,)) {
                        Ok(obj) => {
                            if obj.extract::<String>(py).ok().as_deref() == Some("close") {
                                return Ok(None);
                            }
                            let py_result: &pyo3::Bound<'_, pyo3::types::PyTuple> =
                                obj.downcast_bound(py).unwrap();
                            let key: String = py_result.get_item(0).unwrap().extract().unwrap();
                            let path: String = py_result.get_item(1).unwrap().extract().unwrap();
                            let offset: u64 = py_result.get_item(2).unwrap().extract().unwrap();
                            let length: u64 = py_result.get_item(3).unwrap().extract().unwrap();

                            Ok(Some((key, path, offset, length)))
                        }
                        Err(err) if err.is_instance_of::<Empty>(py) => Err(()),
                        Err(err) => panic!("Error in python process: {:?}", err),
                    },
                );

            // Read result bincode from per-worker file and deserialize — all outside the GIL
            match item {
                Ok(Some((key, path, offset, length))) => {
                    let file = file_cache
                        .entry(path.clone())
                        .or_insert_with(|| File::open(&path).unwrap());
                    file.seek(SeekFrom::Start(offset)).unwrap();
                    result_buffer.resize(length as usize, 0);
                    file.read_exact(&mut result_buffer).unwrap();

                    received += 1;
                    let is_elapsed = last_log_time.elapsed().as_secs() >= 5;
                    if is_elapsed || received % 20 == 0 {
                        if is_elapsed {
                            println!("Python processing... ({received})");
                        } else {
                            println!("Python processing... ({received} pages done)");
                        }
                        last_log_time = std::time::Instant::now();
                    }

                    result_sender
                        .send((key, bincode::deserialize(&result_buffer).unwrap()))
                        .unwrap();
                }
                Ok(None) => break,
                Err(()) => {} // timeout, retry
            }
        }
        println!("Python processing done, received {received} results");
    });

    py_work_queue
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

        let rust_analysis = PageAnalysis::analyse_page(&page.revisions).unwrap();
        let py_analysis = run_analysis_python(py, &page);

        let sentence_rust = {
            let paragraph =
                &rust_analysis[&rust_analysis.revisions_by_id[&2]].paragraphs_ordered[0];
            let sentence_pointer = &rust_analysis[paragraph].sentences_ordered[0];
            &rust_analysis[sentence_pointer]
        };
        let sentence_py = {
            let paragraph = &py_analysis[&py_analysis.revisions_by_id[&2]].paragraphs_ordered[0];
            let sentence_pointer = &py_analysis[paragraph].sentences_ordered[0];
            &py_analysis[sentence_pointer]
        };

        assert_eq!(
            sentence_rust.words_ordered.len(),
            sentence_py.words_ordered.len()
        );

        for (word_rust, word_py) in sentence_rust
            .words_ordered
            .iter()
            .zip(sentence_py.words_ordered.iter())
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
    rust_analysis: &PageAnalysis,
    py_analysis: &PageAnalysis,
) -> Result<(), TestCaseError> {
    prop_assert_eq!(
        py_analysis.ordered_revisions.last().map(|rev| rev.id),
        Some(py_analysis.current_revision.id)
    );

    prop_assert_eq!(
        &rust_analysis
            .ordered_revisions
            .iter()
            .map(|i| i.id)
            .collect::<Vec<_>>(),
        &py_analysis
            .ordered_revisions
            .iter()
            .map(|i| i.id)
            .collect::<Vec<_>>()
    );

    // iterate and compare result graph
    for revision_id in page.revisions.iter().map(|r| r.id) {
        // check spam
        let is_spam_rust = rust_analysis.spam_ids.contains(&revision_id);
        let is_spam_py = py_analysis.spam_ids.contains(&revision_id);
        prop_assert_eq!(is_spam_rust, is_spam_py);

        if is_spam_rust {
            // spam revisions are not analysed further
            continue;
        }
        let input_revision = page.revisions.iter().find(|r| r.id == revision_id).unwrap();
        if input_revision.text.is_empty() {
            // empty revisions are not analysed further
            continue;
        }

        // compare revisions

        let revision_pointer_rust = &rust_analysis.revisions_by_id[&revision_id];
        let revision_pointer_py = &py_analysis.revisions_by_id[&revision_id];

        prop_assert_eq!(revision_pointer_rust.id, revision_pointer_py.id);

        let revision_rust = &rust_analysis[revision_pointer_rust];
        let revision_py = &py_analysis[revision_pointer_py];
        prop_assert_eq!(
            revision_rust.paragraphs_ordered.len(),
            revision_py.paragraphs_ordered.len()
        );
        prop_assert_eq!(revision_rust.original_adds, revision_py.original_adds);

        for (paragraph_pointer_rust, paragraph_pointer_py) in revision_rust
            .paragraphs_ordered
            .iter()
            .zip(revision_py.paragraphs_ordered.iter())
        {
            // compare paragraphs

            prop_assert_eq!(&paragraph_pointer_rust.value, &paragraph_pointer_py.value);

            let paragraph_rust = &rust_analysis[paragraph_pointer_rust];
            let paragraph_py = &py_analysis[paragraph_pointer_py];
            prop_assert_eq!(
                paragraph_rust.sentences_ordered.len(),
                paragraph_py.sentences_ordered.len()
            );

            for (sentence_pointer_rust, sentence_pointer_py) in paragraph_rust
                .sentences_ordered
                .iter()
                .zip(paragraph_py.sentences_ordered.iter())
            {
                // compare sentences

                prop_assert_eq!(&sentence_pointer_rust.value, &sentence_pointer_py.value);

                let sentence_rust = &rust_analysis[sentence_pointer_rust];
                let sentence_py = &py_analysis[sentence_pointer_py];
                prop_assert_eq!(
                    sentence_rust.words_ordered.len(),
                    sentence_py.words_ordered.len()
                );

                for (word_pointer_rust, word_pointer_py) in sentence_rust
                    .words_ordered
                    .iter()
                    .zip(sentence_py.words_ordered.iter())
                {
                    // compare words

                    prop_assert_eq!(&word_pointer_rust.value, &word_pointer_py.value);

                    let word_rust = &rust_analysis[word_pointer_rust];
                    let word_py = &py_analysis[word_pointer_py];
                    prop_assert_eq!(word_pointer_rust.unique_id(), word_pointer_py.unique_id());
                    prop_assert_eq!(
                        &word_rust.inbound.iter().map(|i| i.id).collect::<Vec<_>>(),
                        &word_py.inbound.iter().map(|i| i.id).collect::<Vec<_>>()
                    );
                    prop_assert_eq!(
                        &word_rust.outbound.iter().map(|i| i.id).collect::<Vec<_>>(),
                        &word_py.outbound.iter().map(|i| i.id).collect::<Vec<_>>()
                    );
                    prop_assert_eq!(
                        word_rust.latest_revision.id,
                        word_py.latest_revision.id,
                        "inconsistency at word: {:?}, revision: {}",
                        &word_pointer_rust.value,
                        revision_id
                    );
                    prop_assert_eq!(word_rust.origin_revision.id, word_py.origin_revision.id);
                }
            }
        }
    }
    Ok(())
}

fn compare_algorithm_python(page: &Page) -> Result<(), TestCaseError> {
    with_gil!(py, {
        // run Rust implementation
        let result = PageAnalysis::analyse_page(&page.revisions);
        // reject test case if there are no valid revisions
        prop_assume!(!matches!(result, Err(AnalysisError::NoValidRevisions)));
        let analysis = result.unwrap();

        // run Python implementation
        let wikiwho_py = run_analysis_python(py, page);
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
        #[allow(clippy::question_mark)]
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
        #[allow(clippy::question_mark)]
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

use crate::common::output_structs::page_analysis_from_wikiwho;

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
fn random_pages_100() {
    let reader1 = common::open_test_dump();
    let reader2 = common::open_test_dump();
    let pages = common::pick_n_random_pages((reader1, reader2), 100, 0).unwrap();

    for page in pages {
        compare_algorithm_python(&page).unwrap();
    }
}

#[test]
// this test takes quite some time and especially a LOT of memory (~30GB), could be optimized further if needed
#[ignore]
fn first_1000_pages_mt() {
    const PAGE_COUNT: usize = 1000;

    let reader = common::open_test_dump();
    let pid = std::process::id();
    let temp_path = std::env::temp_dir().join(format!("wikiwho_test_{pid}.bin"));
    let result_dir = std::env::temp_dir().join(format!("wikiwho_test_{pid}_results"));
    let cleanup_path = temp_path.clone();
    let cleanup_result_dir = result_dir.clone();

    // pre-create temp file and result directory
    File::create(&temp_path).unwrap();
    std::fs::create_dir_all(&result_dir).unwrap();

    // python process — returns the work queue for the producer to feed
    let (py_work_queue, py_receiver) = {
        let (result_sender, result_receiver) = std::sync::mpsc::channel();

        let work_queue = Python::with_gil(|py| {
            run_analysis_python_mt(py, result_sender, &temp_path, &result_dir)
        });

        (work_queue, result_receiver)
    };

    // rust thread
    let (rust_sender, rust_receiver) = {
        let (work_sender, work_receiver) = std::sync::mpsc::channel::<PageRef>();
        let (result_sender, result_receiver) = std::sync::mpsc::channel();
        let rust_temp_path = temp_path.clone();

        std::thread::spawn(move || {
            let mut file = File::open(&rust_temp_path).unwrap();
            let mut buf = Vec::new();

            let mut last_log_time = std::time::Instant::now();
            let mut processed = 0;

            for page_ref in work_receiver {
                file.seek(SeekFrom::Start(page_ref.offset)).unwrap();
                buf.resize(page_ref.length as usize, 0);
                file.read_exact(&mut buf).unwrap();
                let page: Page = bincode::deserialize(&buf).unwrap();
                let key = format!("{}:{}", page.namespace, page.title);
                let analysis = PageAnalysis::analyse_page(&page.revisions).unwrap();
                result_sender.send((key, page_ref, analysis)).unwrap();

                processed += 1;

                let is_elapsed = last_log_time.elapsed().as_secs() >= 5;
                if is_elapsed || processed % 20 == 0 {
                    if is_elapsed {
                        println!("Rust processing... ({processed})");
                    } else {
                        println!("Rust processing... ({processed} of {PAGE_COUNT} pages done)");
                    }
                    last_log_time = std::time::Instant::now();
                }
            }

            println!("Rust thread done, processed {processed} pages");
        });

        (work_sender, result_receiver)
    };

    // parser/producer thread — serializes pages to temp file, sends PageRefs to Rust worker
    // and (offset, length) tuples to Python work queue (tiny GIL acquisitions)
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
            rust_sender.send(page_ref).unwrap();
            // Send tiny (offset, length) tuple to Python — minimal GIL time
            Python::with_gil(|py| {
                py_work_queue
                    .call_method1(py, "put_nowait", ((offset, length),))
                    .unwrap();
            });
            offset += length;
        }
        // Signal end of work to Python pool
        Python::with_gil(|py| {
            py_work_queue
                .call_method1(py, "put_nowait", (py.None(),))
                .unwrap();
        });

        println!("Producer thread done, wrote {PAGE_COUNT} pages to temp file");
    });

    // Main matching loop — polls both result channels, compares when both sides are ready.
    // Pages are re-read from the temp file on demand to avoid holding them in memory.
    enum PendingResult {
        RustDone {
            page_ref: PageRef,
            analysis: PageAnalysis,
        },
        PyDone {
            analysis: PageAnalysis,
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
                Ok((key, page_ref, analysis_rust)) => {
                    if let Some(PendingResult::PyDone {
                        analysis: analysis_py,
                    }) = pending.remove(&key)
                    {
                        let page = read_page(&mut main_file, &mut main_buf, page_ref);
                        compare_results(&page, &analysis_rust, &analysis_py).unwrap();
                        compared += 1;
                    } else {
                        pending.insert(
                            key,
                            PendingResult::RustDone {
                                page_ref,
                                analysis: analysis_rust,
                            },
                        );
                    }
                }
                Err(std::sync::mpsc::TryRecvError::Disconnected) => rust_done = true,
                _ => {}
            }
        }
        if !py_done {
            match py_receiver.try_recv() {
                Ok((key, analysis_py)) => {
                    if let Some(PendingResult::RustDone {
                        page_ref,
                        analysis: analysis_rust,
                    }) = pending.remove(&key)
                    {
                        let page = read_page(&mut main_file, &mut main_buf, page_ref);
                        compare_results(&page, &analysis_rust, &analysis_py).unwrap();
                        compared += 1;
                    } else {
                        pending.insert(
                            key,
                            PendingResult::PyDone {
                                analysis: analysis_py,
                            },
                        );
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
    let _ = std::fs::remove_dir_all(&cleanup_result_dir);
}
