// SPDX-License-Identifier: MPL-2.0

use std::{
    collections::HashMap,
    fs::File,
    io::{BufReader, BufWriter, Read},
    path::{Path, PathBuf},
};

use wikiwho::{
    algorithm::{AnalysisError, PageAnalysis, PageAnalysisOptions},
    dump_parser::{DumpParser, Page, ParsingError},
};

mod common;

const ANALYSIS_OPTIONS_RUST: PageAnalysisOptions = PageAnalysisOptions::new();
#[cfg(feature = "python-diff")]
const ANALYSIS_OPTIONS_PY: PageAnalysisOptions = PageAnalysisOptions::new().use_python_diff();

const DATASET2_PARTIAL_DIR: &str = "schuwi-local/docs/dataset2_partial";
const GOLD_STANDARD_PATH: &str = "schuwi-local/docs/gold_standard.partial.newnames.csv";

struct GoldEntry {
    article: String,
    starting_revision: i32,
    token: String,
    context: String,
    correct_origin: i32,
}

/// Splits a single CSV line into fields, handling double-quoted fields
/// (which may contain commas and escaped `""` sequences).
fn parse_csv_fields(line: &str) -> Vec<String> {
    let mut fields = Vec::new();
    let mut field = String::new();
    let mut in_quotes = false;
    let mut chars = line.chars().peekable();

    while let Some(c) = chars.next() {
        match c {
            '"' if !in_quotes => in_quotes = true,
            '"' if in_quotes => {
                if chars.peek() == Some(&'"') {
                    chars.next();
                    field.push('"');
                } else {
                    in_quotes = false;
                }
            }
            ',' if !in_quotes => {
                fields.push(std::mem::take(&mut field));
            }
            _ => field.push(c),
        }
    }
    fields.push(field);
    fields
}

fn parse_gold_standard() -> Vec<GoldEntry> {
    let content = std::fs::read_to_string(GOLD_STANDARD_PATH)
        .unwrap_or_else(|_| panic!("Could not read gold standard at {GOLD_STANDARD_PATH}"));

    let mut entries = Vec::new();

    for line in content.lines().skip(1) {
        if line.trim().is_empty() {
            continue;
        }
        let fields = parse_csv_fields(line);
        if fields.len() < 7 {
            continue;
        }

        // Column 5 is "Correct" — skip rows where it's not a valid integer (e.g., 'x')
        let correct_origin: i32 = match fields[5].trim().parse() {
            Ok(v) => v,
            Err(_) => continue,
        };

        entries.push(GoldEntry {
            article: fields[0].clone(),
            starting_revision: fields[2].trim().parse().expect("invalid revision id"),
            token: fields[3].clone(),
            context: fields[4].clone(),
            correct_origin,
        });
    }

    entries
}

/// Returns the path to the XML file for an article if it exists in the dataset2_partial directory.
fn find_xml_for_article(article: &str) -> Option<PathBuf> {
    let filename = article.replace(' ', "_");
    let path = PathBuf::from(DATASET2_PARTIAL_DIR).join(format!("{filename}.xml"));
    if path.exists() {
        Some(path)
    } else {
        None
    }
}

fn load_page_from_xml(path: &Path) -> Result<Page, ParsingError> {
    let file =
        File::open(path).unwrap_or_else(|e| panic!("Could not open {}: {e}", path.display()));
    let mut parser = DumpParser::new(BufReader::new(file))
        .unwrap_or_else(|e| panic!("Could not create DumpParser for {}: {e}", path.display()));
    Ok(parser
        .parse_page()?
        .unwrap_or_else(|| panic!("No page found in {}", path.display())))
}

/// Additional Wikipedia/Wiktionary dump files to search when an article is not found
/// as a standalone XML file in `DATASET2_PARTIAL_DIR`. Paths starting with `~/` are
/// expanded to the user's home directory. Supported formats: `.bz2`, `.gz`, `.zst`, plain `.xml`.
const EXTRA_DUMPS: &[&str] = &[
    // they are normally distributed by WikiMedia with bzip2 compression but recompressing to zstd
    // (e.g. `bzip2 -dc ....xml.bz2 | zstd -11 -T4 -o ....xml.zst`) makes the scanning
    // in `find_page_in_dump` SO MUCH faster!
    "~/Downloads/enwiki-2026-01-01-p358p689.xml.zst",
    "~/Downloads/enwiki-2026-01-01-p5592p5716.xml.zst",
    "~/Downloads/enwiki-2026-01-01-p7045p7327.xml.zst",
    "~/Downloads/enwiki-2026-01-01-p8121p8239.xml.zst",
    "~/Downloads/enwiki-2026-01-01-p9105p9256.xml.zst",
    "~/Downloads/enwiki-2026-01-01-p9317p9426.xml.zst",
    "~/Downloads/enwiki-2026-01-01-p9428p9592.xml.zst",
    "~/Downloads/enwiki-2026-01-01-p10257p10530.xml.zst",
    "~/Downloads/enwiki-2026-01-01-p24095p24221.xml.zst",
    "~/Downloads/enwiki-2026-01-01-p140332p140700.xml.zst",
    "~/Downloads/enwiki-2026-01-01-p534366p536006.xml.zstd",
    "~/Downloads/enwiki-2026-01-01-p2159021p2164836.xml.zst",
    "~/Downloads/enwiki-2026-01-01-p8704841p8720250.xml.zst",
    "~/Downloads/enwiki-2026-01-01-p8161655p8188573.xml.zst",
];

fn expand_path(path: &str) -> PathBuf {
    if let Some(rest) = path.strip_prefix("~/") {
        let home = std::env::var("HOME").unwrap_or_default();
        PathBuf::from(home).join(rest)
    } else {
        PathBuf::from(path)
    }
}

fn find_page_in_dump(dump_path: &Path, title: &str) -> Option<Page> {
    let file = File::open(dump_path).ok()?;
    let ext = dump_path.extension().and_then(|e| e.to_str()).unwrap_or("");
    let reader: Box<dyn Read> = match ext {
        "bz2" => Box::new(bzip2::read::BzDecoder::new(file)),
        "gz" => Box::new(flate2::read::GzDecoder::new(file)),
        "zst" | "zstd" => Box::new(zstd::stream::Decoder::new(file).ok()?),
        _ => Box::new(file),
    };
    common::find_page_by_title_and_ns(BufReader::new(reader), title, 0)
        .ok()
        .flatten()
}

/// Returns the path to the JSON cache file for an article if it exists.
fn find_cache_for_article(article: &str) -> Option<PathBuf> {
    let filename = article.replace(' ', "_");
    let path = PathBuf::from(DATASET2_PARTIAL_DIR).join(format!("{filename}.json"));
    if path.exists() {
        Some(path)
    } else {
        None
    }
}

fn load_page_from_cache(path: &Path) -> Result<Page, Box<dyn std::error::Error>> {
    let file = File::open(path)?;
    Ok(serde_json::from_reader(BufReader::new(file))?)
}

fn save_page_to_cache(article: &str, page: &Page) {
    let filename = article.replace(' ', "_");
    let path = PathBuf::from(DATASET2_PARTIAL_DIR).join(format!("{filename}.json"));
    let result = File::create(&path)
        .map_err(|e| e.to_string())
        .and_then(|f| serde_json::to_writer(BufWriter::new(f), page).map_err(|e| e.to_string()));
    match result {
        Ok(()) => eprintln!("  Cached '{article}' to {}", path.display()),
        Err(e) => eprintln!("  Warning: failed to cache '{article}': {e}"),
    }
}

/// Loads a page for a gold-standard article: tries standalone XML first,
/// then a JSON cache, then searches `EXTRA_DUMPS` in order (saving to cache on hit).
/// Returns `None` if not found anywhere.
fn load_article_page(article: &str, up_to_rev_inclusive: Option<i32>) -> Option<Page> {
    // Try standalone XML file
    if let Some(xml_path) = find_xml_for_article(article) {
        match (load_page_from_xml(&xml_path), up_to_rev_inclusive) {
            (Ok(page), None) => return Some(page),
            (Ok(mut page), Some(last_rev)) => {
                let cutoff = page.revisions.partition_point(|r| r.id <= last_rev);
                page.revisions.truncate(cutoff);
                return Some(page);
            }
            (Err(e), _) => eprintln!("  Warning: failed to load XML for '{article}': {e}"),
        }
    }

    // Try JSON cache (written after the first dump search to avoid repeating it)
    if let Some(cache_path) = find_cache_for_article(article) {
        match load_page_from_cache(&cache_path) {
            Ok(mut page) => {
                if let Some(last_rev) = up_to_rev_inclusive {
                    let cutoff = page.revisions.partition_point(|r| r.id <= last_rev);
                    page.revisions.truncate(cutoff);
                }
                return Some(page);
            }
            Err(e) => eprintln!("  Warning: failed to load cache for '{article}': {e}"),
        }
    }

    // Fall back to dump files; gold standard uses underscores, Wikipedia titles use spaces
    let title = article.replace('_', " ");
    for &dump_str in EXTRA_DUMPS {
        let dump_path = expand_path(dump_str);
        if !dump_path.exists() {
            continue;
        }
        eprintln!("  Searching for '{title}' in {}…", dump_path.display());
        if let Some(mut page) = find_page_in_dump(&dump_path, &title) {
            if let Some(last_rev) = up_to_rev_inclusive {
                let cutoff = page.revisions.partition_point(|r| r.id <= last_rev);
                page.revisions.truncate(cutoff);
            }
            save_page_to_cache(article, &page);
            return Some(page);
        }
    }

    None
}

/// Tokenizes `context_lower` using the same pipeline as the analysis algorithm:
/// split into paragraphs → sentences (trimmed) → tokens.
///
/// The returned token sequence mirrors what appears in `words_ordered` when
/// the same text is stored in the analysis, making it suitable for sliding-window
/// matching against the article word list.
fn tokenize_as_algorithm(context_lower: &str) -> Vec<String> {
    let mut scratch1 = String::new();
    let mut scratch2 = String::new();
    let mut tokens = Vec::new();

    let paras =
        wikiwho::utils::split_into_paragraphs(context_lower, (&mut scratch1, &mut scratch2));
    for para in &paras {
        let sents =
            wikiwho::utils::split_into_sentences(para.as_ref(), (&mut scratch1, &mut scratch2));
        for sent in sents {
            let sent = wikiwho::utils::trim_in_place(sent);
            if sent.is_empty() {
                continue;
            }
            for tok in wikiwho::utils::split_into_tokens(sent.as_ref()) {
                tokens.push(tok.to_string());
            }
        }
    }

    tokens
}

/// Finds the origin revision of a gold-standard token within the given analysis.
///
/// Tokenizes the context window using the same pipeline as the algorithm, then
/// slides that token sequence over the flat word list of the starting revision.
/// The target token's position within the context determines which matched word
/// to read the origin revision from.
///
/// Returns `None` if the token cannot be located (reported as "not found", not as wrong).
fn find_token_origin(analysis: &PageAnalysis, entry: &GoldEntry) -> Option<i32> {
    let rev_ptr = analysis.revisions_by_id.get(&entry.starting_revision)?;
    let revision = &analysis[rev_ptr];

    // The analysis stores all text lowercased; match accordingly.
    let context_lower = entry.context.to_lowercase();
    let token_lower = entry.token.to_lowercase();

    // Tokenize the context exactly as the algorithm would.
    let context_tokens = tokenize_as_algorithm(&context_lower);

    // Find which position in the context the target token occupies.
    let token_pos = context_tokens.iter().position(|t| t == &token_lower)?;

    // Flatten all words in the starting revision into a single ordered sequence,
    // spanning paragraph and sentence boundaries (the sliding window needs this
    // when the context itself crosses a sentence boundary).
    let all_words: Vec<_> = revision
        .paragraphs_ordered
        .iter()
        .flat_map(|para_ptr| &analysis[para_ptr].sentences_ordered)
        .flat_map(|sent_ptr| &analysis[sent_ptr].words_ordered)
        .collect();

    // Slide the context window over the word sequence and return on the first match.
    let n = context_tokens.len();
    for window in all_words.windows(n) {
        if window
            .iter()
            .zip(context_tokens.iter())
            .skip(1)
            .take(n - 2)
            .all(|(wp, ct)| wp.value.as_str() == ct.as_str())
            && window[0].value.ends_with(&context_tokens[0])
            && window[n - 1].value.starts_with(&context_tokens[n - 1])
        {
            return Some(analysis[window[token_pos]].origin_revision.id);
        }
    }

    None
}

/// Runs the precision evaluation for all gold standard entries that have XML available.
///
/// Returns `(correct, wrong, not_found)` counts.
fn run_precision_test(options: PageAnalysisOptions) -> (usize, usize, usize) {
    let entries = parse_gold_standard();

    let mut by_article: HashMap<String, Vec<&GoldEntry>> = HashMap::new();
    for entry in &entries {
        by_article
            .entry(entry.article.clone())
            .or_default()
            .push(entry);
    }

    let mut correct = 0;
    let mut wrong = 0;
    let mut not_found = 0;

    let mut article_names: Vec<_> = by_article.keys().collect();
    article_names.sort();

    for article in article_names {
        let article_entries = &by_article[article];

        // Truncate revisions at the highest starting_revision used by any gold entry
        // for this article. This avoids processing years of revisions added after the
        // gold standard was created (which can be enormous for popular articles).
        let max_starting_rev = article_entries
            .iter()
            .map(|e| e.starting_revision)
            .max()
            .unwrap();

        let Some(page) = load_article_page(article, Some(max_starting_rev)) else {
            eprintln!("  Skipping '{article}': not found in XML dir or any configured dump");
            continue;
        };

        println!(
            "  Evaluating '{article}' ({} entries)…",
            article_entries.len()
        );

        let analysis = match PageAnalysis::analyse_page_with_options(&page.revisions, options) {
            Ok(a) => a,
            Err(AnalysisError::NoValidRevisions) => {
                eprintln!("  Skipping '{article}': no valid revisions");
                continue;
            }
            Err(e) => panic!("Analysis failed for '{article}': {e}"),
        };

        for entry in article_entries {
            match find_token_origin(&analysis, entry) {
                None => {
                    eprintln!(
                        "    NOT FOUND: token '{}' (context: '{}')",
                        entry.token, entry.context
                    );
                    not_found += 1;
                }
                Some(attributed) => {
                    if attributed == entry.correct_origin {
                        correct += 1;
                    } else {
                        eprintln!(
                            "    WRONG: token '{}' attributed to rev {}, expected {}",
                            entry.token, attributed, entry.correct_origin
                        );
                        wrong += 1;
                    }
                }
            }
        }
    }

    (correct, wrong, not_found)
}

/// Tests the accuracy of the pure-Rust (imara-diff/histogram) algorithm against the
/// paper's gold standard dataset.
///
/// The original WikiWho paper reports ~95% precision on the full gold standard.
/// We evaluate on the subset for which XML revision histories are available locally
/// (see `schuwi-local/docs/dataset2_partial/`).
#[test]
fn gold_standard_precision_rust() {
    let (correct, wrong, not_found) = run_precision_test(ANALYSIS_OPTIONS_RUST);
    let total_evaluated = correct + wrong;
    assert!(
        total_evaluated > 0,
        "No gold standard entries could be evaluated — are the XML files in {DATASET2_PARTIAL_DIR}?"
    );
    let precision = correct as f64 / total_evaluated as f64;
    println!(
        "Pure-Rust precision: {correct}/{total_evaluated} = {:.1}% ({not_found} tokens not located)",
        precision * 100.0
    );
    assert!(
        precision >= 0.85,
        "Expected ≥85% precision, got {:.1}% ({correct}/{total_evaluated})",
        precision * 100.0
    );
}

/// Baseline: tests the python-diff backend against the same gold standard.
///
/// Since `algorithm_exact_tests.rs` confirms python-diff matches the original WikiWho
/// exactly, this acts as a sanity check that our token lookup logic is reasonable.
/// The original paper reports ~95% precision on the full 240-token gold standard;
/// our limited subset (≤18 tokens from 3 articles) and text-based context matching
/// may score lower due to ambiguous common tokens ("in", "the") appearing many times.
#[test]
#[cfg(feature = "python-diff")]
fn gold_standard_precision_python_diff() {
    let (correct, wrong, not_found) = run_precision_test(ANALYSIS_OPTIONS_PY);
    let total_evaluated = correct + wrong;
    assert!(
        total_evaluated > 0,
        "No gold standard entries could be evaluated — are the XML files in {DATASET2_PARTIAL_DIR}?"
    );
    let precision = correct as f64 / total_evaluated as f64;
    println!(
        "Python-diff precision: {correct}/{total_evaluated} = {:.1}% ({not_found} tokens not located)",
        precision * 100.0
    );
    // if we had the full gold standard we should be able to do >= 90%
    assert!(
        precision >= 0.85,
        "Expected ≥85% precision for python-diff baseline, got {:.1}% ({correct}/{total_evaluated})",
        precision * 100.0
    );
}

/// Compares word-level origin attribution between pure-Rust and python-diff at the
/// final revision of each dataset2_partial article.
///
/// Reports the agreement rate: the fraction of words for which both backends attribute
/// the same origin revision. Disagreements indicate cases where imara-diff's different
/// LCS choices produce a different attribution than Python's difflib.
#[test]
#[cfg(feature = "python-diff")]
fn divergence_rate_dataset2_partial() {
    let entries = parse_gold_standard();

    let mut by_article: HashMap<String, Vec<&GoldEntry>> = HashMap::new();
    for entry in &entries {
        by_article
            .entry(entry.article.clone())
            .or_default()
            .push(entry);
    }

    let mut article_names: Vec<_> = by_article.keys().collect();
    article_names.sort();

    // const PATHS: &[&str] = &[
    //     "schuwi-local/docs/dataset2_partial/Amstrad_CPC.xml",
    //     "schuwi-local/docs/dataset2_partial/Antarctica.xml",
    //     "schuwi-local/docs/dataset2_partial/Apollo_11.xml",
    // ];

    // let reader1 = common::open_test_dump();
    // let reader2 = common::open_test_dump();
    // let pages: Vec<Page> = common::pick_n_random_pages((reader1, reader2), 100, 5).unwrap();

    let mut total_agree = 0usize;
    let mut total_words = 0usize;

    for article in article_names {
        let article_entries = &by_article[article];

        // Truncate revisions at the highest starting_revision used by any gold entry
        // for this article. This avoids processing years of revisions added after the
        // gold standard was created (which can be enormous for popular articles).
        let max_starting_rev = article_entries
            .iter()
            .map(|e| e.starting_revision)
            .max()
            .unwrap();

        let Some(page) = load_article_page(article, Some(max_starting_rev)) else {
            eprintln!("  Skipping '{article}': not found in XML dir or any configured dump");
            continue;
        };

        println!(
            "  Evaluating '{article}' ({} entries)…",
            article_entries.len()
        );

        // for page in pages {
        let (rust_analysis, py_analysis) = std::thread::scope(|s| {
            let rust_analysis = s.spawn(|| {
                PageAnalysis::analyse_page_with_options(&page.revisions, ANALYSIS_OPTIONS_RUST)
                    .unwrap()
            });
            let py_analysis = s.spawn(|| {
                PageAnalysis::analyse_page_with_options(&page.revisions, ANALYSIS_OPTIONS_PY)
                    .unwrap()
            });

            (rust_analysis.join().unwrap(), py_analysis.join().unwrap())
        });

        let final_rev_id = rust_analysis.current_revision.id;
        let rust_rev = &rust_analysis[&rust_analysis.revisions_by_id[&final_rev_id]];
        let py_rev = &py_analysis[&py_analysis.revisions_by_id[&final_rev_id]];

        let mut agree = 0usize;
        let mut total = 0usize;

        for (para_rust, para_py) in rust_rev
            .paragraphs_ordered
            .iter()
            .zip(py_rev.paragraphs_ordered.iter())
        {
            for (sent_rust, sent_py) in rust_analysis[para_rust]
                .sentences_ordered
                .iter()
                .zip(py_analysis[para_py].sentences_ordered.iter())
            {
                for (word_rust, word_py) in rust_analysis[sent_rust]
                    .words_ordered
                    .iter()
                    .zip(py_analysis[sent_py].words_ordered.iter())
                {
                    let origin_rust = rust_analysis[word_rust].origin_revision.id;
                    let origin_py = py_analysis[word_py].origin_revision.id;
                    total += 1;
                    if origin_rust == origin_py {
                        agree += 1;
                    }
                }
            }
        }

        let agreement_rate = agree as f64 / total as f64;
        println!(
            "{}: {agree}/{total} words agree ({:.2}%)",
            page.title,
            agreement_rate * 100.0
        );

        total_agree += agree;
        total_words += total;
    }

    let overall_rate = total_agree as f64 / total_words as f64;
    println!(
        "Overall: {total_agree}/{total_words} words agree ({:.2}%)",
        overall_rate * 100.0
    );
    assert!(
        overall_rate >= 0.85,
        "Expected ≥85% precision for Rust vs. Python divergence rate, got {:.1}% ({total_agree}/{total_words})",
        overall_rate * 100.0
    );
}
