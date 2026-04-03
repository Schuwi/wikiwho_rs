use std::borrow::Borrow;
// SPDX-License-Identifier: MPL-2.0
use std::collections::HashMap;
use std::io::{self, BufRead, BufReader, BufWriter, Write};
use std::process::ExitCode;
use std::rc::Rc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use yoke::Yoke;

use wikiwho::algorithm::PageAnalysis;
use wikiwho::dump_parser::{Contributor, DumpParser, Namespace, Page, Revision};
use wikiwho::utils::iterate_revision_tokens;

/// Formats a `Contributor` directly into the serializer, avoiding an intermediate `String`
/// allocation per editor field. Output: user id as string, or `"0|username"` for IP/anonymous.
fn serialize_editor<S: serde::Serializer>(
    contributor: &&Contributor,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    match contributor.id {
        Some(id) if id != 0 => serializer.collect_str(&id),
        _ => serializer.collect_str(&format_args!("0|{}", contributor.username)),
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Format {
    Jsonl,
    Json,
    Raw,
}

fn print_usage(program: &str) {
    eprintln!(
        "Usage: {program} [OPTIONS] [INPUT]

Runs the WikiWho authorship attribution algorithm on a MediaWiki XML dump.

Arguments:
  INPUT                   Input XML dump file (omit or \"-\" for stdin)
                          Compression auto-detected from extension (.bz2, .zst, .gz)

Options:
  -o, --output PATH       Output file (omit or \"-\" for stdout)
                          Compression auto-detected from extension (.bz2, .zst, .gz)
  -f, --format FORMAT     Output format: jsonl (default), json, raw
  -j, --jobs N            Number of worker threads (default: number of CPUs)
  -n, --namespace NS      Only process pages in this namespace (repeatable)
  -N, --pages N           Only process the first N pages
  -q, --quiet             Suppress progress messages on stderr
  -h, --help              Show this help message"
    );
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("Error: {e}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let mut opts = getopts::Options::new();
    opts.optopt(
        "o",
        "output",
        "Output file (\"-\" or omit for stdout)",
        "PATH",
    );
    opts.optopt(
        "f",
        "format",
        "Output format: jsonl (default), json, raw",
        "FORMAT",
    );
    opts.optopt("j", "jobs", "Number of worker threads", "N");
    opts.optmulti(
        "n",
        "namespace",
        "Only process pages in this namespace",
        "NS",
    );
    opts.optopt("N", "limit", "Limit the number of pages to process", "N");
    opts.optflag("q", "quiet", "Suppress progress messages on stderr");
    opts.optflag("h", "help", "Show help");

    let args: Vec<String> = std::env::args().collect();
    let matches = opts.parse(&args[1..]).map_err(|e| e.to_string())?;

    if matches.opt_present("h") {
        print_usage(&args[0]);
        return Ok(());
    }

    let format = match matches.opt_str("f").as_deref() {
        None | Some("jsonl") => Format::Jsonl,
        Some("json") => Format::Json,
        Some("raw") => Format::Raw,
        Some(other) => return Err(format!("unknown format: {other}").into()),
    };

    let num_threads: usize = match matches.opt_str("j") {
        Some(s) => {
            let n = s
                .parse::<usize>()
                .map_err(|e| format!("invalid -j value: {e}"))?;
            if n == 0 {
                return Err("--jobs must be at least 1".into());
            }
            n
        }
        None => std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(1),
    };

    let namespace_filter: Vec<i32> = matches
        .opt_strs("n")
        .iter()
        .map(|s| s.parse::<i32>())
        .collect::<Result<_, _>>()
        .map_err(|e| format!("invalid namespace: {e}"))?;

    let limit_pages: Option<u64> = matches.opt_get("N")?;

    let quiet = matches.opt_present("q");

    let input_path = matches.free.first().map(|s| s.as_str());
    let output_path = matches.opt_str("o");

    // Set up input reader with auto-decompression
    let reader: Box<dyn BufRead + Send> = match input_path {
        None | Some("-") => Box::new(BufReader::new(io::stdin())),
        Some(path) => {
            let file = std::fs::File::open(path)
                .map_err(|e| format!("cannot open input '{path}': {e}"))?;
            if path.ends_with(".bz2") {
                Box::new(BufReader::new(bzip2::read::BzDecoder::new(file)))
            } else if path.ends_with(".zst") || path.ends_with(".zstd") {
                Box::new(BufReader::new(
                    zstd::Decoder::new(file).map_err(|e| format!("zstd init: {e}"))?,
                ))
            } else if path.ends_with(".gz") {
                Box::new(BufReader::new(flate2::read::GzDecoder::new(file)))
            } else {
                Box::new(BufReader::new(file))
            }
        }
    };

    // Set up output writer with auto-compression
    let writer: Box<dyn Write> = match output_path.as_deref() {
        None | Some("-") => Box::new(BufWriter::new(io::stdout().lock())),
        Some(path) => {
            let file = std::fs::File::create(path)
                .map_err(|e| format!("cannot create output '{path}': {e}"))?;
            if path.ends_with(".bz2") {
                Box::new(BufWriter::new(bzip2::write::BzEncoder::new(
                    file,
                    bzip2::Compression::default(),
                )))
            } else if path.ends_with(".zst") || path.ends_with(".zstd") {
                Box::new(BufWriter::new(
                    zstd::Encoder::new(file, 3).map_err(|e| format!("zstd init: {e}"))?,
                ))
            } else if path.ends_with(".gz") {
                Box::new(BufWriter::new(flate2::write::GzEncoder::new(
                    file,
                    flate2::Compression::default(),
                )))
            } else {
                Box::new(BufWriter::new(file))
            }
        }
    };

    if num_threads == 1 {
        process_single(
            reader,
            writer,
            format,
            &namespace_filter,
            quiet,
            limit_pages,
        )
    } else {
        process_parallel(
            reader,
            writer,
            format,
            &namespace_filter,
            quiet,
            limit_pages,
            num_threads,
        )
    }
}

// ---------------------------------------------------------------------------
// Formatting helpers
// ---------------------------------------------------------------------------

fn format_duration(d: Duration) -> String {
    let secs = d.as_secs();
    if secs < 10 {
        format!("{}.{}s", secs, d.subsec_millis() / 100)
    } else if secs < 60 {
        format!("{secs}s")
    } else if secs < 3600 {
        format!("{}m {:02}s", secs / 60, secs % 60)
    } else {
        format!("{}h {:02}m", secs / 3600, (secs % 3600) / 60)
    }
}

fn format_bytes(bytes: u64) -> String {
    const KIB: f64 = 1024.0;
    const MIB: f64 = 1024.0 * KIB;
    const GIB: f64 = 1024.0 * MIB;
    const TIB: f64 = 1024.0 * GIB;
    let b = bytes as f64;
    if b >= TIB {
        format!("{:.1} TiB", b / TIB)
    } else if b >= GIB {
        format!("{:.1} GiB", b / GIB)
    } else if b >= MIB {
        format!("{:.1} MiB", b / MIB)
    } else if b >= KIB {
        format!("{:.0} KiB", b / KIB)
    } else {
        format!("{bytes} B")
    }
}

fn format_count(n: u64) -> String {
    let s = n.to_string();
    let mut result = String::with_capacity(s.len() + s.len() / 3);
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i) % 3 == 0 {
            result.push(',');
        }
        result.push(c);
    }
    result
}

// ---------------------------------------------------------------------------
// Progress reporting
// ---------------------------------------------------------------------------

const REPORT_INTERVAL_MS: u64 = 2_000;

struct ProgressReporter {
    quiet: bool,
    start: Instant,
    last_status_ms: AtomicU64,
    namespaces: HashMap<i32, Namespace>,

    parsed_pages: AtomicU64,
    parsed_text_bytes: AtomicU64,
    parsed_revisions: AtomicU64,
    analysed_pages: AtomicU64,
    skipped_pages: AtomicU64,
    written_pages: AtomicU64,
}

impl ProgressReporter {
    fn new(quiet: bool, namespaces: HashMap<i32, Namespace>) -> Self {
        Self {
            quiet,
            start: Instant::now(),
            last_status_ms: AtomicU64::new(0),
            namespaces,
            parsed_pages: AtomicU64::new(0),
            parsed_text_bytes: AtomicU64::new(0),
            parsed_revisions: AtomicU64::new(0),
            analysed_pages: AtomicU64::new(0),
            skipped_pages: AtomicU64::new(0),
            written_pages: AtomicU64::new(0),
        }
    }

    /// Record a parsed page. Warns on stderr about very large pages (>1 GiB).
    fn page_parsed(&self, page: &Page) {
        let total_text_bytes: usize = page
            .revisions
            .iter()
            .map(|rev| rev.text.as_str().len())
            .sum();
        let num_revisions = page.revisions.len() as u64;

        if !self.quiet && total_text_bytes > 1024 * 1024 * 1024 {
            self.warn_big_page(page, total_text_bytes);
        }

        self.parsed_text_bytes
            .fetch_add(total_text_bytes as u64, Ordering::Relaxed);
        self.parsed_revisions
            .fetch_add(num_revisions, Ordering::Relaxed);
        self.parsed_pages.fetch_add(1, Ordering::Relaxed);
    }

    fn warn_big_page(&self, page: &Page, total_text_bytes: usize) {
        let parsed = self.parsed_pages.load(Ordering::Relaxed);
        let ns_name = self
            .namespaces
            .get(&page.namespace)
            .map(|ns| match ns {
                Namespace::Default => "[Default]".to_string(),
                Namespace::Named(name) => format!("'{name}'"),
            })
            .unwrap_or_else(|| "[Unknown]".to_string());

        let avg_info = if parsed > 0 {
            let avg_bytes = self.parsed_text_bytes.load(Ordering::Relaxed) as f64 / parsed as f64;
            let avg_revs = self.parsed_revisions.load(Ordering::Relaxed) as f64 / parsed as f64;
            format!(
                " (avg: {}, {:.1} revs over {} pages)",
                format_bytes(avg_bytes as u64),
                avg_revs,
                format_count(parsed),
            )
        } else {
            String::new()
        };

        eprintln!(
            "  Warning: big page '{}' (ns {} {}): {}, {} revisions{}",
            page.title,
            page.namespace,
            ns_name,
            format_bytes(total_text_bytes as u64),
            format_count(page.revisions.len() as u64),
            avg_info,
        );
    }

    fn page_analysed(&self) {
        self.analysed_pages.fetch_add(1, Ordering::Relaxed);
    }

    fn page_skipped(&self, title: &str, err: &str) {
        self.skipped_pages.fetch_add(1, Ordering::Relaxed);
        if !self.quiet {
            eprintln!("  Warning: skipping '{title}': {err}");
        }
    }

    /// Record a written page. Prints a periodic status line (~every 2s).
    fn page_written(&self) {
        let written = self.written_pages.fetch_add(1, Ordering::Relaxed) + 1;
        if self.quiet {
            return;
        }

        let elapsed_ms = self.start.elapsed().as_millis() as u64;
        let last = self.last_status_ms.load(Ordering::Relaxed);
        if elapsed_ms.saturating_sub(last) >= REPORT_INTERVAL_MS {
            self.last_status_ms.store(elapsed_ms, Ordering::Relaxed);
            self.print_status(written);
        }
    }

    fn print_status(&self, written: u64) {
        let elapsed = self.start.elapsed();
        let rate = written as f64 / elapsed.as_secs_f64().max(0.001);
        let parsed_bytes = self.parsed_text_bytes.load(Ordering::Relaxed);
        let skipped = self.skipped_pages.load(Ordering::Relaxed);

        let skip_part = if skipped > 0 {
            format!(", {} skipped", format_count(skipped))
        } else {
            String::new()
        };

        eprintln!(
            "  [{}] {} pages written ({:.0}/s), {} parsed{}",
            format_duration(elapsed),
            format_count(written),
            rate,
            format_bytes(parsed_bytes),
            skip_part,
        );
    }

    fn finish(&self) {
        if self.quiet {
            return;
        }
        let elapsed = self.start.elapsed();
        let written = self.written_pages.load(Ordering::Relaxed);
        let rate = written as f64 / elapsed.as_secs_f64().max(0.001);
        let parsed_bytes = self.parsed_text_bytes.load(Ordering::Relaxed);
        let revisions = self.parsed_revisions.load(Ordering::Relaxed);
        let skipped = self.skipped_pages.load(Ordering::Relaxed);

        let skip_part = if skipped > 0 {
            format!(", {} skipped", format_count(skipped))
        } else {
            String::new()
        };

        eprintln!(
            "  Done in {}: {} pages written ({:.0}/s), {} parsed, {} revisions{}",
            format_duration(elapsed),
            format_count(written),
            rate,
            format_bytes(parsed_bytes),
            format_count(revisions),
            skip_part,
        );
    }
}

struct RevisionIterHelper<'a>(Rc<&'a mut Revision>);

impl<'a> Borrow<Revision> for RevisionIterHelper<'a> {
    fn borrow(&self) -> &Revision {
        &self.0
    }
}

fn text_deleting_iterator(
    revisions: &mut [Revision],
) -> impl Iterator<Item = RevisionIterHelper<'_>> {
    revisions.iter_mut().scan(None, |state, rev| {
        let this_rev = Rc::new(rev);
        let last_rev = std::mem::replace(state, Some(this_rev.clone()));
        if let Some(last_rev) = last_rev {
            if let Some(last_rev) = Rc::into_inner(last_rev) {
                // we don't use the text again, so we might as well drop the original to save memory
                last_rev.text = wikiwho::dump_parser::Text::Deleted;
            } else {
                // this is just an optimization so if it fails that does not impact correctness
                // but we want to be alerted to this during debugging
                debug_assert!(false, "Revision was not dropped by analyse_page after processing, can't free old text");
            }
        }
        Some(RevisionIterHelper(this_rev))
    })
}

/// Single-threaded processing (original path, used when -j 1).
fn process_single(
    reader: Box<dyn BufRead + Send>,
    mut writer: Box<dyn Write>,
    format: Format,
    namespace_filter: &[i32],
    quiet: bool,
    page_limit: Option<u64>,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut parser =
        DumpParser::new(reader).map_err(|e| format!("failed to initialize parser: {e:?}"))?;
    let reporter = ProgressReporter::new(quiet, parser.site_info().namespaces.clone());

    if !quiet {
        eprintln!("Database: {}", parser.site_info().dbname);
    }

    let mut page_count: u64 = 0;

    if format == Format::Json {
        write!(writer, "[")?;
    }

    while let Some(mut page) = parser
        .parse_page()
        .map_err(|e| format!("XML parse error: {e:?}"))?
    {
        if !namespace_filter.is_empty() && !namespace_filter.contains(&page.namespace) {
            continue;
        }

        if page_limit.map(|n| page_count >= n).unwrap_or(false) {
            break;
        }

        reporter.page_parsed(&page);

        let analysis = match PageAnalysis::analyse_page(text_deleting_iterator(&mut page.revisions))
        {
            Ok(a) => a,
            Err(e) => {
                reporter.page_skipped(&page.title, &e.to_string());
                continue;
            }
        };

        reporter.page_analysed();
        let yoke = Yoke::attach_to_cart(Box::new((page, analysis)), |cart| {
            build_page_output(&cart.0, &cart.1)
        });
        write_page_result(&mut writer, &yoke, format, page_count)?;
        reporter.page_written();
        page_count += 1;
    }

    if format == Format::Json {
        writeln!(writer, "]")?;
    }

    writer.flush()?;
    reporter.finish();

    Ok(())
}

/// Result from a worker thread: either a pre-built output or a skipped page.
///
/// The `Ok` variant carries a [`PageOutput`] yoked to its backing `(Page, PageAnalysis)`.
/// Workers build the `PageOutput` (resolving pointers, collecting token data) so that the
/// writer thread only needs to serialize — no construction work on the hot I/O path.
enum AnalysisResult {
    Ok(Yoke<PageOutput<'static>, Box<(Page, PageAnalysis)>>),
    Skipped(String, String), // (title, error message)
}

/// Multi-threaded processing pipeline:
///
/// ```text
///   parser (1 thread) --> workers (N threads) --> writer (main thread)
///          |    bounded channel    |    bounded channel    |
/// ```
///
/// Both channels are bounded (`num_threads * 2`), which makes the pipeline self-balancing:
/// if the writer is slow, the result channel fills → workers block → work channel fills →
/// parser blocks. No stage can overwhelm another, and memory usage stays bounded.
///
/// Workers build [`PageOutput`] (via [`build_page_output`]) before sending results, so the
/// writer thread only calls `serde_json::to_writer` — keeping I/O throughput high.
///
/// Output order is non-deterministic (no reordering). This avoids head-of-line blocking
/// where one slow large page would hold up all completed pages behind it.
fn process_parallel(
    reader: Box<dyn BufRead + Send>,
    mut writer: Box<dyn Write>,
    format: Format,
    namespace_filter: &[i32],
    quiet: bool,
    page_limit: Option<u64>,
    num_threads: usize,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut parser =
        DumpParser::new(reader).map_err(|e| format!("failed to initialize parser: {e:?}"))?;
    let reporter = ProgressReporter::new(quiet, parser.site_info().namespaces.clone());

    if !quiet {
        eprintln!("Database: {}", parser.site_info().dbname);
        eprintln!("Processing with {} threads", num_threads);
    }

    // Bounded channel: parser -> workers (back-pressure to avoid unbounded memory growth)
    let (work_tx, work_rx) = std::sync::mpsc::sync_channel::<Page>(num_threads * 2);
    let work_rx = Mutex::new(work_rx);

    // Bounded channel: workers -> writer (back-pressure prevents memory growth)
    let (result_tx, result_rx) = std::sync::mpsc::sync_channel::<AnalysisResult>(num_threads * 2);

    // Track errors from the parser thread
    let parse_error: Mutex<Option<String>> = Mutex::new(None);

    std::thread::scope(|s| {
        // Spawn N worker threads that pull pages and analyze them
        for n in 0..num_threads {
            let work_rx = &work_rx;
            let result_tx = result_tx.clone();
            let reporter = &reporter;
            std::thread::Builder::new()
                .name(format!("worker {n}"))
                .spawn_scoped(s, move || {
                    loop {
                        let item = work_rx.lock().unwrap().recv();
                        let mut page = match item {
                            Ok(v) => v,
                            Err(_) => break, // channel closed, no more work
                        };

                        let result = match PageAnalysis::analyse_page(text_deleting_iterator(
                            &mut page.revisions,
                        )) {
                            Ok(analysis) => {
                                reporter.page_analysed();
                                let yoke =
                                    Yoke::attach_to_cart(Box::new((page, analysis)), |cart| {
                                        build_page_output(&cart.0, &cart.1)
                                    });
                                AnalysisResult::Ok(yoke)
                            }
                            Err(e) => {
                                AnalysisResult::Skipped(page.title.to_string(), e.to_string())
                            }
                        };
                        // If the writer has dropped, stop
                        if result_tx.send(result).is_err() {
                            break;
                        }
                    }
                })
                .unwrap();
        }

        // Drop the original result_tx so that result_rx closes once all workers finish
        drop(result_tx);

        // Parser thread: reads pages sequentially, applies namespace filter, sends to workers
        let parse_error_ref = &parse_error;
        let reporter = &reporter;
        std::thread::Builder::new()
            .name("parser".to_string())
            .spawn_scoped(s, move || {
                let mut parsed_count = 0u64;
                loop {
                    match parser.parse_page() {
                        Ok(Some(page)) => {
                            if !namespace_filter.is_empty()
                                && !namespace_filter.contains(&page.namespace)
                            {
                                continue;
                            }
                            if page_limit.map(|n| parsed_count >= n).unwrap_or(false) {
                                break; // page limit reached
                            }
                            reporter.page_parsed(&page);
                            if work_tx.send(page).is_err() {
                                break; // workers gone
                            }
                            parsed_count += 1;
                        }
                        Ok(None) => break, // end of stream
                        Err(e) => {
                            *parse_error_ref.lock().unwrap() =
                                Some(format!("XML parse error: {e:?}"));
                            break;
                        }
                    }
                }
                // work_tx is dropped here, signaling workers to finish
            })
            .unwrap();

        // Main thread: collect results and write (no reordering)
        if format == Format::Json {
            write!(writer, "[").unwrap();
        }

        let mut page_count: u64 = 0;

        for result in &result_rx {
            match result {
                AnalysisResult::Skipped(title, err) => {
                    reporter.page_skipped(&title, &err);
                }
                AnalysisResult::Ok(yoke) => {
                    write_page_result(&mut writer, &yoke, format, page_count).unwrap();
                    page_count += 1;
                    reporter.page_written();
                }
            }
            // TODO: drop in separate thread
        }

        if format == Format::Json {
            writeln!(writer, "]").unwrap();
        }

        writer.flush().unwrap();
        reporter.finish();
    });

    // Check if the parser hit an error
    if let Some(err) = parse_error.into_inner().unwrap() {
        return Err(err.into());
    }

    Ok(())
}

fn write_page_result(
    writer: &mut Box<dyn Write>,
    yoke: &Yoke<PageOutput<'static>, Box<(Page, PageAnalysis)>>,
    format: Format,
    page_count: u64,
) -> Result<(), Box<dyn std::error::Error>> {
    // TODO: look into speeding up serialization
    match format {
        Format::Jsonl => {
            serde_json::to_writer(&mut *writer, yoke.get())?;
            writeln!(writer)?;
        }
        Format::Json => {
            if page_count > 0 {
                write!(writer, ",")?;
            }
            serde_json::to_writer(&mut *writer, yoke.get())?;
        }
        Format::Raw => {
            serde_json::to_writer(&mut *writer, &yoke.backing_cart().1)?;
            writeln!(writer)?;
        }
    }
    Ok(())
}

/// Serialization-ready view of a page's analysis results.
///
/// These structs derive `Serialize` so `serde_json` can stream fields directly to the writer
/// without building an intermediate `serde_json::Value` tree — avoiding a full duplicate of
/// all token strings, revision ids, and editor data in memory during serialization.
///
/// Fields borrow from `Page` and `PageAnalysis` (e.g. `&'a str` for token values,
/// `&'a Contributor` for editors). The `Yokeable` derive allows these borrowing structs to
/// be sent across threads: workers build a `PageOutput` and yoke it to a `Box<(Page,
/// PageAnalysis)>` cart, which keeps the borrowed data alive.
#[derive(serde::Serialize, yoke::Yokeable)]
struct PageOutput<'a> {
    article_title: &'a str,
    namespace: i32,
    revisions: Vec<RevisionOutput<'a>>,
    spam_ids: &'a [i32],
    all_tokens: Vec<TokenOutput<'a>>,
}

#[derive(serde::Serialize, yoke::Yokeable)]
struct RevisionOutput<'a> {
    id: i32,
    timestamp: String,
    #[serde(serialize_with = "serialize_editor")]
    editor: &'a Contributor,
}

#[derive(serde::Serialize, yoke::Yokeable)]
struct TokenOutput<'a> {
    token_id: usize,
    #[serde(rename = "str")]
    value: &'a str,
    o_rev_id: i32,
    #[serde(serialize_with = "serialize_editor")]
    editor: &'a Contributor,
    #[serde(rename = "in")]
    inbound: Vec<i32>,
    #[serde(rename = "out")]
    outbound: Vec<i32>,
}

/// Builds a [`PageOutput`] that borrows from both the parsed `Page` and the `PageAnalysis`.
///
/// This resolves internal pointers (revision ids, token origins, in/out edges) into a flat
/// structure ready for serialization. Called on worker threads so the writer thread doesn't
/// need to do this work.
fn build_page_output<'a>(page: &'a Page, analysis: &'a PageAnalysis) -> PageOutput<'a> {
    let revisions_by_id: HashMap<i32, &Revision> =
        page.revisions.iter().map(|rev| (rev.id, rev)).collect();

    let revisions: Vec<RevisionOutput<'a>> = analysis
        .ordered_revisions
        .iter()
        .map(|rev_ptr| {
            let xml_revision = revisions_by_id[&rev_ptr.id];
            RevisionOutput {
                id: xml_revision.id,
                timestamp: xml_revision.timestamp.to_rfc3339(),
                editor: &xml_revision.contributor,
            }
        })
        .collect();

    let last_rev = &analysis.current_revision;
    let all_tokens: Vec<TokenOutput<'a>> = iterate_revision_tokens(analysis, last_rev)
        .map(|word_ptr| {
            let word_analysis = &analysis[word_ptr];
            let xml_origin_revision = revisions_by_id[&word_analysis.origin_revision.id];
            TokenOutput {
                token_id: word_ptr.unique_id(),
                value: &word_ptr.value,
                o_rev_id: xml_origin_revision.id,
                editor: &xml_origin_revision.contributor,
                inbound: word_analysis.inbound.iter().map(|r| r.id).collect(),
                outbound: word_analysis.outbound.iter().map(|r| r.id).collect(),
            }
        })
        .collect();

    PageOutput {
        article_title: &page.title,
        namespace: page.namespace,
        revisions,
        spam_ids: &analysis.spam_ids,
        all_tokens,
    }
}
