// SPDX-License-Identifier: MPL-2.0
//! Shared test helpers.
//!
//! The pyo3-based helpers (everything that talks to the reference Python WikiWho) are
//! gated behind the `python-diff` feature; the pure-Rust helpers (`open_test_dump`,
//! `find_page_by_title_and_ns`, `pick_n_random_pages`, `proptest_support`, …) compile
//! without it, so tests that don't need Python (e.g. `gold_standard_precision_rust` in
//! `algorithm_statistic_tests.rs`) can use them. Python-dependent tests still require a
//! venv with `requirements.txt` installed.
#![allow(unused)]

use chrono::DateTime;
use memchr::memmem::Finder;
use rand::{prelude::IndexedRandom, seq::SliceRandom, SeedableRng};
use std::{
    collections::HashMap,
    fs::File,
    io::{BufRead, BufReader, Read},
};

use wikiwho::dump_parser::{self, Contributor, Page, Revision, Text};

#[cfg(feature = "python-diff")]
use pyo3::prelude::*;

#[cfg(feature = "python-diff")]
pub mod prelude {
    pub(crate) use super::proptest_support;
    pub(crate) use super::{dummy_revision, with_gil};
    pub(crate) use proptest::prelude::*;
    pub(crate) use pyo3::prelude::*;
}

#[cfg(feature = "serde")]
pub use delta_debugging::delta_debug_texts;

#[cfg(feature = "python-diff")]
macro_rules! with_gil {
    ($py: ident, $body: expr) => {{
        let result = Python::attach(|$py| {
            let _: () = $body;
            Ok(())
        });
        // workaround for prop_assert! not working correctly in Python::with_gil
        #[allow(clippy::question_mark)]
        if result.is_err() {
            return result;
        }
    }};
}
#[cfg(feature = "python-diff")]
pub(crate) use with_gil;

#[cfg(feature = "serde")]
pub(crate) fn bincode_serialize<T: serde::Serialize>(value: &T) -> Vec<u8> {
    bincode::serde::encode_to_vec(value, bincode::config::standard()).unwrap()
}

#[cfg(feature = "serde")]
pub(crate) fn bincode_deserialize<T: serde::de::DeserializeOwned>(
    bytes: &[u8],
) -> Result<T, bincode::error::DecodeError> {
    bincode::serde::decode_from_slice(bytes, bincode::config::standard()).map(|(value, _)| value)
}

pub fn dummy_revision() -> Revision {
    Revision {
        id: 0,
        text: Text::Deleted,
        timestamp: DateTime::from_timestamp_nanos(0),
        contributor: Contributor {
            id: None,
            username: "Dummy".into(),
        },
        comment: None,
        sha1: None,
        minor: false,
    }
}

/// Note that the propability distribution for picking a page is not uniform
/// but proportional to the size of it's XML representation. I.e. larger pages
/// are more likely to be picked.
///
/// May return less than `n` pages.
///
/// The input is a tuple of two `BufRead` instances that should be equal.
/// That's because the function needs to do two passes over the input to pick the pages.
pub fn pick_n_random_pages<P: PageRepresentation, R: std::io::Read>(
    full_xml: (R, R),
    n: usize,
    seed: u64,
) -> Result<Vec<P>, P::Error> {
    // First pass: find page start offsets
    const PAGE_START: &[u8] = b"<page";
    const SEARCH_BUFFER_LEN: usize = 8192 + PAGE_START.len();

    let mut buffer = Box::new([0; SEARCH_BUFFER_LEN]);
    let mut buffered_bytes = 0;

    let mut reader = full_xml.0;
    let mut page_starts = Vec::new();
    let mut stream_offset = 0;

    let finder = Finder::new(PAGE_START);

    loop {
        // fill buffer from reader
        let empty_buf = &mut buffer[buffered_bytes..];
        let read = reader.read(empty_buf)?;
        buffered_bytes += read;

        let last_iteration = read == 0;

        if !last_iteration && buffer.len() < PAGE_START.len() {
            // buffer is not full and we need more data to find a page start
            continue;
        }

        let consume;

        let buf = &buffer[..buffered_bytes];
        if let Some(start) = finder.find(buf) {
            page_starts.push(stream_offset + start as u64);

            consume = start + PAGE_START.len();
        } else {
            // No page start found in this buffer

            // keep the last few bytes to find a potential page start that spans two buffers
            // (saturating so a buffer smaller than PAGE_START doesn't underflow)
            consume = buf.len().saturating_sub(PAGE_START.len());
        }
        stream_offset += consume as u64;
        buffer.copy_within(consume..buffered_bytes, 0);
        buffered_bytes -= consume;

        if last_iteration {
            break;
        }
    }

    // pick n random pages

    /* define specific algorithm to ensure reproducibility */
    let mut rng = rand_xoshiro::Xoshiro256PlusPlus::seed_from_u64(seed);
    let mut chosen_offsets: Vec<_> = page_starts.sample(&mut rng, n).copied().collect();
    chosen_offsets.sort_unstable();

    // Second pass: parse the chosen pages
    let mut pages = Vec::new();

    let mut current_offset = 0;
    let mut reader = BufReader::new(full_xml.1);

    for offset in chosen_offsets {
        // skip to the start of the page
        loop {
            let buf = reader.fill_buf()?;
            if buf.is_empty() {
                break;
            }
            let offset_diff = offset - current_offset;

            if buf.len() as u64 >= offset_diff {
                current_offset += offset_diff;
                reader.consume(offset_diff as usize);
                break;
            } else {
                let consume = buf.len();
                current_offset += consume as u64;
                reader.consume(consume);
            }
        }

        let mut read_bytes = 0;
        let page = P::from_xml(&mut reader, &mut read_bytes)?;
        current_offset += read_bytes as u64;
        pages.push(page);
    }

    Ok(pages)
}

/// Find a page by title and namespace in a full XML dump.
///
/// The function uses a regex to find the page start tag and then parses the page.
///
/// It may fail to find the page if the XML is formatted in an unexpected way.
///
/// # Arguments
/// * `full_xml` - A reader for the full XML dump
/// * `title` - The title of the page to find
/// * `namespace` - The namespace id of the page to find
///
/// # Returns
/// The page if found, otherwise `None`
///
/// # Errors
/// * May error if the XML is malformed or an IO error occurs
pub fn find_page_by_title_and_ns<P: PageRepresentation, R: std::io::Read>(
    full_xml: R,
    title: &str,
    namespace: i32,
) -> Result<Option<P>, P::Error> {
    // the expected maximum length of a match in bytes
    // (includes page start tag, title and any potential whitespace in between)
    const MAXIMUM_MATCH_LEN: usize = 512;
    const SEARCH_BUFFER_LEN: usize = 8192 + MAXIMUM_MATCH_LEN;
    let page_regex = regex::bytes::Regex::new(&format!(
        r#"<page[^>]*>[^<]*<title[^>]*>([^:<]+:)?{}</title>[^<]*<ns[^>]*>{}</ns>"#,
        title, namespace
    ))
    .unwrap();
    let mut buffer = Box::new([0; SEARCH_BUFFER_LEN]);
    let mut buffered_bytes = 0;
    let mut reader = full_xml;

    loop {
        // fill buffer from reader
        let empty_buf = &mut buffer[buffered_bytes..];
        let read = reader.read(empty_buf)?;
        buffered_bytes += read;

        let last_iteration = read == 0;

        if !last_iteration && buffer.len() < MAXIMUM_MATCH_LEN {
            // buffer is not full and we need more data to find a page start
            continue;
        }

        if let Some(m) = page_regex.find(&buffer[..buffered_bytes]) {
            let start = m.start();

            let buf = &buffer[start..buffered_bytes];

            return Ok(Some(P::from_xml(
                buf.chain(BufReader::new(reader)),
                &mut 0,
            )?));
        } else {
            // No page start found in this buffer

            // keep the last few bytes to find a potential page start that spans two buffers
            // (saturating so a file smaller than MAXIMUM_MATCH_LEN doesn't underflow)
            let consume = buffered_bytes.saturating_sub(MAXIMUM_MATCH_LEN);
            buffer.copy_within(consume..buffered_bytes, 0);
            buffered_bytes -= consume;
        }

        if last_iteration {
            break;
        }
    }

    Ok(None)
}

/// Opens the reference dump used by the real-page parity tests.
///
/// The dump location can be overridden with the `WIKIWHO_TEST_DUMP` environment
/// variable. CI uses this to point at a small representative subset on pull requests
/// and at the full dump on pushes to `main`; locally it defaults to the gitignored
/// reference dump under `dev-data/`.
///
/// Returns `None` (after printing a `SKIP:` notice) when the dump is not present, so
/// callers can return early instead of panicking. Use the `let Some(reader) = … else
/// { return; }` pattern at the call site.
pub fn open_test_dump() -> Option<impl Read> {
    const DEFAULT_DUMP_PATH: &str =
        "dev-data/reference-dumps/dewiktionary-20240901-pages-meta-history.xml.zst";

    let path = std::env::var("WIKIWHO_TEST_DUMP").unwrap_or_else(|_| DEFAULT_DUMP_PATH.to_owned());

    match File::open(&path) {
        Ok(file) => Some(
            zstd::stream::Decoder::new(file).expect("failed to create zstd decoder for test dump"),
        ),
        Err(err) => {
            eprintln!(
                "SKIP: reference dump not available at `{path}` ({err}); set WIKIWHO_TEST_DUMP \
                 to a dump file to run real-page parity tests."
            );
            None
        }
    }
}

pub trait PageRepresentation: Sized {
    type Error: From<std::io::Error>;
    fn from_xml<R: BufRead>(xml_reader: R, read_bytes: &mut usize) -> Result<Self, Self::Error>;
}

impl PageRepresentation for Page {
    type Error = dump_parser::ParsingError;

    fn from_xml<R: BufRead>(xml_reader: R, read_bytes: &mut usize) -> Result<Self, Self::Error> {
        dump_parser::DumpParser::parse_single_page(xml_reader, read_bytes)
    }
}

impl PageRepresentation for Vec<u8> {
    type Error = std::io::Error;

    fn from_xml<R: BufRead>(
        mut xml_reader: R,
        read_bytes: &mut usize,
    ) -> Result<Self, Self::Error> {
        let mut xml = Vec::new();

        // read bytes until </page> is found
        let mut buf = Vec::new();
        loop {
            xml_reader.read_until(b'>', &mut buf)?;
            xml.extend_from_slice(&buf);
            if buf.ends_with(b"</page>") {
                break;
            }
            buf.clear();
        }

        *read_bytes = xml.len();

        Ok(xml)
    }
}

#[cfg(feature = "python-diff")]
pub fn load_local_module<'a>(py: Python<'a>, module_name: &str) -> PyResult<Bound<'a, PyModule>> {
    // make sure the current directory is in sys.path
    py.run(
        c"
import sys
if '' not in sys.path:
    sys.path.append('')
        ",
        None,
        None,
    )?;

    let module = PyModule::import(py, module_name)?;
    Ok(module)
}

#[allow(clippy::useless_conversion)] // pyo3 proc macros generate identity conversions for PyErr
#[cfg(all(feature = "serde", feature = "python-diff"))]
pub mod output_structs {
    use super::*;
    use pyo3::{
        types::{PyBytes, PyDict, PyList, PyString},
        FromPyObject,
    };
    use std::sync::Arc;
    use wikiwho::algorithm::{
        ArcSubstring, PageAnalysis, ParagraphImmutables, ParagraphPointer, RevisionAnalysis,
        RevisionImmutables, RevisionPointer, SentenceImmutables, SentencePointer, WordAnalysis,
        WordImmutables, WordPointer,
    };

    #[pyfunction]
    pub fn serialize_wikiwho_result<'a>(
        py_wikiwho: &Bound<'a, PyAny>,
        page_bincode: &Bound<'a, PyBytes>,
    ) -> PyResult<Bound<'a, PyBytes>> {
        let page: Page = bincode_deserialize(page_bincode.as_bytes()).map_err(|err| {
            pyo3::exceptions::PyValueError::new_err(format!(
                "Invalid page_bincode. Expected bincode-serialized Page. Error: {err}"
            ))
        })?;

        let page_analysis = page_analysis_from_wikiwho(py_wikiwho, &page)?;
        let serialized = bincode_serialize(&page_analysis);
        Ok(PyBytes::new(py_wikiwho.py(), &serialized))
    }

    pub fn page_analysis_from_wikiwho(
        py_wikiwho: &Bound<'_, PyAny>,
        page: &dump_parser::Page,
    ) -> PyResult<PageAnalysis> {
        // iterate over values in Python dict like this: {_key: [value, ..], ...}
        // calls closure with unique Python object id of each `value` and extracted `value` (`T`)
        fn py_iter_ht<T>(
            hashtable: &Bound<'_, PyDict>,
            mut f: impl FnMut(usize, T) -> PyResult<()>,
        ) -> PyResult<()>
        where
            T: for<'a, 'py> FromPyObject<'a, 'py, Error = PyErr>,
        {
            for (_hash, list) in hashtable.iter() {
                let list: Bound<PyList> = list.cast_into()?;

                for obj in list.iter() {
                    let obj_id = obj.as_ptr().addr();
                    let extracted: T = obj.extract()?;

                    f(obj_id, extracted)?;
                }
            }

            Ok(())
        }

        let py = py_wikiwho.py();
        let py_wikiwho: PyWikiwho = py_wikiwho.extract()?;

        let dummy_revision = (RevisionAnalysis::default(), RevisionImmutables::dummy());
        let mut result = PageAnalysis::new(dummy_revision);

        // maps the wikipedia revision id to a pointer
        let mut revision_pointers: HashMap<i32, RevisionPointer> = HashMap::new();

        // maps the unique Python object id to a pointer
        let mut paragraph_id_pointers: HashMap<usize, ParagraphPointer> = HashMap::new();
        let mut sentence_id_pointers: HashMap<usize, SentencePointer> = HashMap::new();
        let mut token_id_pointers: HashMap<usize, WordPointer> = HashMap::new();

        // Use XML page to populate revisions
        for revision in &page.revisions {
            let pointer = result.new_revision(RevisionImmutables::from_revision(revision));
            revision_pointers.insert(revision.id, pointer.clone());
        }

        // Use `paragraphs_ht`, `sentences_ht` and `tokens` attrs to build all our pointers
        py_iter_ht(
            py_wikiwho.paragraphs_ht.bind(py),
            |py_paragraph_id, py_paragraph: PyParagraph| {
                let pointer = result.new_paragraph(ParagraphImmutables::new(
                    ArcSubstring::new_source(Arc::new(py_paragraph.value.to_str(py)?.to_string())),
                ));

                paragraph_id_pointers.insert(py_paragraph_id, pointer);
                Ok(())
            },
        )?;

        py_iter_ht(
            py_wikiwho.sentences_ht.bind(py),
            |py_sentence_id, py_sentence: PySentence| {
                let pointer = result.new_sentence(SentenceImmutables::new(
                    ArcSubstring::new_source(Arc::new(py_sentence.value.to_str(py)?.to_string())),
                ));

                sentence_id_pointers.insert(py_sentence_id, pointer);
                Ok(())
            },
        )?;

        for py_token_obj in py_wikiwho.tokens.bind(py).iter() {
            let py_token: PyWord = py_token_obj.extract()?;
            let word_data = WordImmutables::new(ArcSubstring::new_source(Arc::new(
                py_token.value.to_str(py)?.to_string(),
            )));
            let mut word_analysis = WordAnalysis::new(&revision_pointers[&py_token.origin_rev_id]);

            // Let's already populate `WordAnalysis` while we're at it
            word_analysis.latest_revision = revision_pointers[&py_token.last_rev_id].clone();

            for py_id in py_token.inbound.bind(py).iter() {
                let rev_id: i32 = py_id.extract()?;
                let pointer = revision_pointers[&rev_id].clone();
                word_analysis.inbound.push(pointer);
            }

            for py_id in py_token.outbound.bind(py).iter() {
                let rev_id: i32 = py_id.extract()?;
                let pointer = revision_pointers[&rev_id].clone();
                word_analysis.outbound.push(pointer);
            }

            let pointer = result.new_word(word_data, word_analysis);
            result.words.push(pointer.clone());
            token_id_pointers.insert(py_token_obj.as_ptr().addr(), pointer);
        }

        // Populate `RevisionAnalysis`, `ParagraphAnalysis` and `SentenceAnalysis`
        fn populate_analysis_children_ht<T>(
            py_ordered_hashes: &Bound<PyList>,
            py_ht: &Bound<PyDict>,
            pointers: &HashMap<usize, T>,
        ) -> PyResult<Vec<T>>
        where
            T: Clone,
        {
            let mut disambiguation_map: HashMap<String, usize> = HashMap::new();

            let mut result = Vec::new();
            for py_hash in py_ordered_hashes.iter() {
                let hash: String = py_hash.extract()?;
                let py_ht_entry: Bound<PyList> = py_ht
                    .get_item(py_hash)?
                    .ok_or_else(|| {
                        pyo3::exceptions::PyValueError::new_err(format!(
                            "Hash {hash} not found in hashtable"
                        ))
                    })?
                    .cast_into()?;

                // if there are multiple children with the same hash, then they are ordered by
                // appearance in the ordered list and we can disambiguate them by counting
                // how many times we've seen the same hash before
                let index = *disambiguation_map
                    .entry(hash)
                    .and_modify(|index| *index += 1)
                    .or_default();
                let py_obj = py_ht_entry.get_item(index)?;

                let py_id = py_obj.as_ptr().addr();
                let pointer = pointers[&py_id].clone();
                result.push(pointer);
            }
            Ok(result)
        }

        py_iter_ht(
            py_wikiwho.sentences_ht.bind(py),
            |py_sentence_id, py_sentence: PySentence| {
                let sentence_ptr = &sentence_id_pointers[&py_sentence_id];
                let sentence_analysis = &mut result[sentence_ptr];

                let py_words = py_sentence.words.bind(py);
                for py_word in py_words.iter() {
                    let py_id = py_word.as_ptr().addr();
                    let pointer = token_id_pointers[&py_id].clone();
                    sentence_analysis.words_ordered.push(pointer);
                }
                Ok(())
            },
        )?;

        py_iter_ht(
            py_wikiwho.paragraphs_ht.bind(py),
            |py_paragraph_id, py_paragraph: PyParagraph| {
                let paragraph_ptr = &paragraph_id_pointers[&py_paragraph_id];
                let paragraph_analysis = &mut result[paragraph_ptr];

                let py_sentence_hashes = py_paragraph.ordered_sentences;
                paragraph_analysis.sentences_ordered = populate_analysis_children_ht(
                    py_sentence_hashes.bind(py),
                    py_paragraph.sentences.bind(py),
                    &sentence_id_pointers,
                )?;
                Ok(())
            },
        )?;

        for (_, py_revision_obj) in py_wikiwho.revisions.bind(py).iter() {
            let py_revision: PyRevision = py_revision_obj.extract()?;
            let revision_ptr = &revision_pointers[&py_revision.id];
            let revision_analysis = &mut result[revision_ptr];

            revision_analysis.original_adds = py_revision.original_adds;

            let py_paragraphs = py_revision.ordered_paragraphs;
            revision_analysis.paragraphs_ordered = populate_analysis_children_ht(
                py_paragraphs.bind(py),
                py_revision.paragraphs.bind(py),
                &paragraph_id_pointers,
            )?;

            result
                .revisions_by_id
                .insert(py_revision.id, revision_ptr.clone());
        }

        // Copy simple fields
        for py_revision in py_wikiwho.ordered_revisions.bind(py).iter() {
            let py_revid: i32 = py_revision.extract()?;
            let revision_pointer = revision_pointers[&py_revid].clone();
            result.ordered_revisions.push(revision_pointer);
        }

        result.spam_ids = py_wikiwho.spam_ids;
        result.current_revision = revision_pointers[&py_wikiwho.revision_curr.id].clone();

        Ok(result)
    }

    #[derive(FromPyObject)]
    struct PyWikiwho {
        // title: Py<PyString>,
        revisions: Py<PyDict>, /* {int: PyRevision, ...} - key = wikipedia revision id */
        paragraphs_ht: Py<PyDict>, /* {string: [PyParagraph, ..], ...} - key = hash */
        sentences_ht: Py<PyDict>, /* {string: [PySentence, ..], ...} - key = hash */
        tokens: Py<PyList>,    /* [PyWord, ...] */

        spam_ids: Vec<i32>,

        ordered_revisions: Py<PyList>, /* [int, ...] - wikipedia revision ids */
        revision_curr: PyRevision,
    }

    #[derive(FromPyObject)]
    struct PyRevision {
        id: i32,
        paragraphs: Py<PyDict>, /* {string: [PyParagraph, ..], ...} - key = hash */
        ordered_paragraphs: Py<PyList>, /* [string, ...] - hashes */
        original_adds: usize,
    }

    #[derive(FromPyObject)]
    struct PyParagraph {
        value: Py<PyString>,
        sentences: Py<PyDict>, /* {string: [PySentence, ..], ...} - key = hash */
        ordered_sentences: Py<PyList>, /* [string, ...] - hashes */
    }

    #[derive(FromPyObject)]
    struct PySentence {
        value: Py<PyString>,
        words: Py<PyList>, /* [PyWord, ...] */
    }

    #[derive(FromPyObject)]
    struct PyWord {
        value: Py<PyString>,
        origin_rev_id: i32,
        last_rev_id: i32,
        outbound: Py<PyList>, /* [int, ...] - revision ids */
        inbound: Py<PyList>,  /* [int, ...] - revision ids */
    }
}

#[allow(clippy::useless_conversion)] // pyo3 proc macros generate identity conversions for PyErr
#[cfg(all(feature = "serde", feature = "python-diff"))]
pub mod input_structs {
    use super::*;
    use pyo3::{prelude::*, IntoPyObjectExt};

    macro_rules! with_pickle_functions {
        (#[pymethods] impl $name:ident { $($other:tt)* }) => {
            #[pymethods]
            impl $name {
                #[new]
                fn new() -> Self {
                    Self::default()
                }

                pub fn __getstate__(&self, py: Python) -> PyResult<Py<PyAny>> {
                    Ok(pyo3::types::PyBytes::new(py, &bincode_serialize(self)).into_any().unbind())
                }

                pub fn __setstate__(&mut self, _py: Python, state: Py<PyAny>) -> PyResult<()> {
                    let state = state.extract::<&[u8]>(_py).unwrap();
                    *self = bincode_deserialize(state).unwrap();
                    Ok(())
                }

                pub fn __reduce__(slf: &pyo3::Bound<Self>) -> PyResult<Py<PyAny>> {
                    let py = slf.py();
                    let state = slf.borrow().__getstate__(py)?;
                    let cls = slf.get_type();
                    let new_fn = cls.getattr(pyo3::intern!(py, "__new__"))?;
                    (new_fn, (cls,), state).into_py_any(py)
                }

                $(
                    $other
                )*
            }
        }
    }

    #[pyclass(module = "tests.support", from_py_object)]
    #[derive(Clone, Default, serde::Serialize, serde::Deserialize)]
    pub struct PyDeleted(bool);

    with_pickle_functions! {
        #[pymethods]
        impl PyDeleted {
            #[getter]
            fn text(&self) -> bool {
                self.0
            }

            #[getter]
            fn restricted(&self) -> bool {
                // not relevant for algorithm behavior
                false
            }
        }
    }

    #[pyclass(module = "tests.support", from_py_object)]
    #[derive(Clone, Default, serde::Serialize, serde::Deserialize)]
    pub struct PyTimestamp;

    with_pickle_functions! {
        #[pymethods]
        impl PyTimestamp {
            fn long_format(&self) -> String {
                // not relevant for algorithm behavior
                "".to_string()
            }
        }
    }

    #[pyclass(module = "tests.support", from_py_object)]
    #[derive(Clone, Default, serde::Serialize, serde::Deserialize)]
    pub struct PyRevision {
        #[pyo3(get)]
        pub id: i32,
        #[pyo3(get)]
        pub text: Option<String>,
        #[pyo3(get)]
        pub comment: Option<String>,
        #[pyo3(get)]
        pub sha1: Option<String>,
        #[pyo3(get)]
        pub minor: bool,
        #[pyo3(get)]
        pub deleted: PyDeleted,
        #[pyo3(get)]
        pub timestamp: PyTimestamp,
    }

    impl PyRevision {
        pub fn from_revision(rev: &Revision) -> Self {
            let (text, deleted) = match &rev.text {
                Text::Normal(s) => (Some(s.clone()), PyDeleted(false)),
                Text::Deleted => (None, PyDeleted(true)),
            };
            Self {
                id: rev.id,
                text,
                comment: rev.comment.as_ref().map(|s| s.to_string()),
                sha1: rev.sha1.as_ref().map(|s| {
                    std::str::from_utf8(&s.0)
                        .expect("sha1 to be base36 encoded")
                        .to_string()
                }),
                minor: rev.minor,
                deleted,
                timestamp: PyTimestamp,
            }
        }
    }

    with_pickle_functions! {
        #[pymethods]
        impl PyRevision {
            #[getter]
            fn user(&self) -> Option<()> {
                // not relevant for algorithm behavior
                None
            }
        }
    }

    #[pyclass(module = "tests.support", from_py_object)]
    #[derive(Clone, Default, serde::Serialize, serde::Deserialize)]
    pub struct PyPage {
        #[pyo3(get)]
        pub title: String,
        #[pyo3(get)]
        pub namespace: i32,
        pub revisions: Vec<PyRevision>,
    }

    with_pickle_functions! {
        #[pymethods]
        impl PyPage {
            fn __iter__<'a>(this: &Bound<'a, Self>) -> PyResult<Bound<'a, PyAny>> {
                let py = this.py();
                let revisions = this.borrow().revisions.clone();

                revisions.into_bound_py_any(py)?.call_method0("__iter__")
            }

            #[staticmethod]
            fn from_bincode(state: &[u8]) -> PyResult<Self> {
                let page: wikiwho::dump_parser::Page = match bincode_deserialize(state) {
                    Ok(page) => page,
                    Err(err) => return Err(pyo3::exceptions::PyValueError::new_err(
                        format!("Invalid state for PyPage. Expected bincode-serialized Page. Error: {err}"),
                    )),
                };

                Ok(PyPage::from_page(&page))
            }
        }
    }

    impl PyPage {
        pub fn from_page(page: &wikiwho::dump_parser::Page) -> Self {
            Self {
                title: page.title.to_string(),
                namespace: page.namespace,
                revisions: page
                    .revisions
                    .iter()
                    .map(PyRevision::from_revision)
                    .collect(),
            }
        }
    }
}

pub mod proptest_support {
    use compact_str::CompactString;
    use proptest::prelude::*;
    use proptest::strategy::Strategy;

    use wikiwho::dump_parser::{Contributor, Page, Revision, Sha1Hash, Text};

    pub fn maybe_comment() -> impl Strategy<Value = Option<CompactString>> {
        prop_oneof![
            7 => Just(None),
            1 => any::<String>().prop_map(CompactString::from).prop_map(Some)
        ]
    }

    pub fn correct_text(text_strategy: BoxedStrategy<String>) -> impl Strategy<Value = Text> {
        prop_oneof![
            1 => Just(Text::Deleted),
            3 => text_strategy.prop_map(Text::Normal)
        ]
    }

    pub fn sha1(text: &Text) -> impl Strategy<Value = Sha1Hash> {
        match text {
            Text::Deleted => Just(Sha1Hash(*b"verycoolhashofdeletedtext123456")),
            Text::Normal(text) => {
                // Just use any hash function here, only needs to make sure the same text always has the same hash
                // Collisions are not a concern since we have "few" revisions in our tests
                let hash = blake3::Hasher::new().update(text.as_bytes()).finalize();
                let hash_as_hex = hex::encode(hash.as_bytes());
                Just(Sha1Hash(hash_as_hex.as_bytes()[..31].try_into().unwrap()))
            }
        }
    }

    pub fn maybe_sha1(text: &Text, has_hash: bool) -> impl Strategy<Value = Option<Sha1Hash>> {
        if has_hash {
            sha1(text).prop_map(Some).boxed()
        } else {
            Just(None).boxed()
        }
    }

    prop_compose! {
        pub fn correct_revision(id: i32, has_hash: bool, text_strategy: BoxedStrategy<String>)
                (text in correct_text(text_strategy))
                (sha1 in maybe_sha1(&text, has_hash), text in Just(text), comment in maybe_comment(), minor in proptest::bool::weighted(0.125))
        -> Revision {
            Revision {
                id, /* must be unique */
                timestamp: chrono::DateTime::from_timestamp_nanos(0), /* ignored in algorithm */
                contributor: Contributor { /* ignored in algorithm */
                    id: None,
                    username: "".into(),
                },
                text,
                sha1,
                comment,
                minor
            }
        }
    }

    pub fn correct_revision_vec(
        has_hash: bool,
        text_strategy: BoxedStrategy<String>,
        max_revisions: i32,
    ) -> impl Strategy<Value = Vec<Revision>> {
        (1..max_revisions).prop_flat_map(move |num_revisions| {
            let mut revisions = Vec::new();
            for i in 0..num_revisions {
                revisions.push(correct_revision(i + 1, has_hash, text_strategy.clone()));
            }
            revisions
        })
    }

    prop_compose! {
        pub fn correct_page(text_strategy: BoxedStrategy<String>, max_revisions: i32)
                (has_hash in proptest::bool::weighted(0.8))
                (revisions in correct_revision_vec(has_hash, text_strategy.clone(), max_revisions))
        -> Page {
            Page {
                title: "Pagetitle".into(), /* ignored in algorithm */
                namespace: 0, /* ignored in algorithm */
                revisions
            }
        }
    }
}

#[cfg(feature = "serde")]
pub mod delta_debugging {
    use std::collections::HashSet;

    use wikiwho::{
        dump_parser::{Page, Text},
        // test_support::page_to_xml,
    };

    fn simplify_text(text: &str) -> Vec<String> {
        let mut candidates = Vec::new();

        // Remove characters one by one
        let chars = text.chars();
        let num_chars = chars.clone().count();
        for i in 0..num_chars {
            let simplified = chars
                .clone()
                .enumerate()
                .filter_map(|(j, c)| if i == j { None } else { Some(c) })
                .collect::<String>();
            candidates.push(simplified);
        }

        // Remove words one by one
        for word in text.split_whitespace() {
            let simplified = text
                .replacen(word, "", 1)
                .replace("  ", " ")
                .trim()
                .to_string();
            candidates.push(simplified);
        }

        // Shorten the string by halves
        if num_chars > 1 {
            let half = num_chars / 2;
            candidates.push(chars.clone().take(half).collect());
            candidates.push(chars.skip(half).collect());
        }

        candidates
    }

    fn simplify_both_texts(text_a: &str, text_b: &str) -> Vec<(String, String)> {
        let mut candidates = Vec::new();

        // Remove characters from both texts
        for i in 0..text_a.len().min(text_b.len()) {
            let simplified_a = format!("{}{}", &text_a[..i], &text_a[i + 1..]);
            let simplified_b = format!("{}{}", &text_b[..i], &text_b[i + 1..]);
            candidates.push((simplified_a, simplified_b));
        }

        // Remove words from both texts
        let words_a: Vec<&str> = text_a.split_whitespace().collect();
        let words_b: Vec<&str> = text_b.split_whitespace().collect();
        for (word_a, word_b) in words_a.iter().zip(words_b.iter()) {
            let simplified_a = text_a
                .replacen(word_a, "", 1)
                .replace("  ", " ")
                .trim()
                .to_string();
            let simplified_b = text_b
                .replacen(word_b, "", 1)
                .replace("  ", " ")
                .trim()
                .to_string();
            candidates.push((simplified_a, simplified_b));
        }

        // Shorten both strings by halves
        if text_a.len() > 1 && text_b.len() > 1 {
            let half_a = text_a.len() / 2;
            let half_b = text_b.len() / 2;
            candidates.push((text_a[..half_a].to_string(), text_b[..half_b].to_string()));
            candidates.push((text_a[half_a..].to_string(), text_b[half_b..].to_string()));
        }

        candidates
    }

    fn simplify_individually(page: &Page) -> Vec<Page> {
        let mut reduced_pages = Vec::new();

        for (i, rev) in page.revisions.iter().enumerate() {
            // Only simplify Normal text
            if let Text::Normal(text) = &rev.text {
                let simplifications = simplify_text(text);
                for simplified_text in simplifications {
                    let mut new_page = page.clone();
                    new_page.revisions[i].text = Text::Normal(simplified_text.clone());
                    reduced_pages.push(new_page);
                }
            }
        }

        reduced_pages
    }

    fn simplify_jointly(page: &Page) -> Vec<Page> {
        let mut reduced_pages = Vec::new();

        if page.revisions.len() != 2 {
            return reduced_pages; // Ensure exactly two revisions
        }

        let rev1 = &page.revisions[0];
        let rev2 = &page.revisions[1];

        if let (Text::Normal(text1), Text::Normal(text2)) = (&rev1.text, &rev2.text) {
            let simplifications = simplify_both_texts(text1, text2);
            for (simplified_text1, simplified_text2) in simplifications {
                let mut new_page = page.clone();
                new_page.revisions[0].text = Text::Normal(simplified_text1.clone());
                new_page.revisions[1].text = Text::Normal(simplified_text2.clone());
                reduced_pages.push(new_page);
            }
        }

        reduced_pages
    }

    fn apply_individual_simplifications(
        current_page: &Page,
        test_page: impl Fn(&Page) -> bool,
        iterations: &mut usize,
    ) -> Option<Page> {
        let candidates = simplify_individually(current_page);
        for candidate in candidates {
            *iterations += 1;
            if test_page(&candidate) {
                println!(
                    "Simplified individually: {}",
                    serde_json::to_string(&candidate).unwrap()
                );
                return Some(candidate);
            }
        }
        None
    }

    fn apply_joint_simplifications(
        current_page: &Page,
        test_page: impl Fn(&Page) -> bool,
        iterations: &mut usize,
    ) -> Option<Page> {
        let candidates = simplify_jointly(current_page);
        for candidate in candidates {
            *iterations += 1;
            if test_page(&candidate) {
                #[cfg(test)]
                println!(
                    "Simplified jointly: {}",
                    serde_json::to_string(&candidate).unwrap()
                );
                return Some(candidate);
            }
        }
        None
    }

    /// Try to simplify a known-failing page by removing characters, words, or splitting the text in half.
    ///
    /// The `test_page` function should return `true` if the simplified page is still failing.
    ///
    /// # Arguments
    /// * `current_page` - The page to simplify
    /// * `test_page` - A function that tests if the simplified page is still failing
    /// * `max_iterations` - A rough limit on how often to call `test_page` before giving up
    ///
    /// # Returns
    /// The simplified page if a simplification was successful, otherwise the original page
    pub fn delta_debug_texts(
        mut current_page: Page,
        test_page: impl Fn(&Page) -> bool,
        max_iterations: usize,
    ) -> Page {
        let mut changed = true;
        let mut visited = HashSet::new();
        let mut iterations = 0;

        // sanity check
        iterations += 1;
        if !test_page(&current_page) {
            return current_page;
        }

        while changed && iterations < max_iterations {
            changed = false;

            // Serialize current_page to check for revisits
            if visited.contains(&current_page) {
                println!("Reached an already visited page.");
                break; // Already visited
            }
            visited.insert(current_page.clone());

            // Phase 2: Simplify Individually
            if let Some(new_page) =
                apply_individual_simplifications(&current_page, &test_page, &mut iterations)
            {
                current_page = new_page;
                changed = true;
                continue;
            }

            // Phase 3: Simplify Jointly
            if let Some(new_page) =
                apply_joint_simplifications(&current_page, &test_page, &mut iterations)
            {
                current_page = new_page;
                changed = true;
                continue;
            }

            // If no changes were made, terminate
        }

        if iterations >= max_iterations {
            println!("Reached maximum iterations.");
        }

        current_page
    }
}
