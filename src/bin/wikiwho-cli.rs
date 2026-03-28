// SPDX-License-Identifier: MPL-2.0
use std::collections::{BTreeMap, HashMap};
use std::io::{self, BufRead, BufReader, BufWriter, Write};
use std::process::ExitCode;
use std::sync::Mutex;

use wikiwho::algorithm::PageAnalysis;
use wikiwho::dump_parser::{Contributor, DumpParser, Page, Revision};
use wikiwho::utils::iterate_revision_tokens;

fn format_editor(contributor: &Contributor) -> String {
    match contributor.id {
        Some(id) if id != 0 => id.to_string(),
        _ => format!("0|{}", contributor.username),
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

    if !quiet {
        let info = parser.site_info();
        eprintln!("Database: {}", info.dbname);
    }

    let mut page_count: u64 = 0;

    if format == Format::Json {
        write!(writer, "[")?;
    }

    while let Some(page) = parser
        .parse_page()
        .map_err(|e| format!("XML parse error: {e:?}"))?
    {
        if !namespace_filter.is_empty() && !namespace_filter.contains(&page.namespace) {
            continue;
        }

        if page_limit.map(|n| page_count >= n).unwrap_or(false) {
            break; // page limit reached
        }

        let analysis = match PageAnalysis::analyse_page(&page.revisions) {
            Ok(a) => a,
            Err(e) => {
                if !quiet {
                    eprintln!("Warning: skipping '{}': {e}", page.title);
                }
                continue;
            }
        };

        let page_title = page.title.clone();
        write_page_result(&mut writer, page, &analysis, format, page_count)?;

        page_count += 1;
        if !quiet && page_count % 100 == 0 {
            eprintln!("Processed {page_count} pages (latest: {})", page_title);
        }
    }

    if format == Format::Json {
        writeln!(writer, "]")?;
    }

    writer.flush()?;

    if !quiet {
        eprintln!("Done. Processed {page_count} pages.");
    }

    Ok(())
}

/// Result from a worker thread: either a successful analysis or a skipped page.
enum AnalysisResult {
    Ok(Page, PageAnalysis),
    Skipped(String, String), // (title, error message)
}

/// Multi-threaded processing pipeline:
///   parser (1 thread) -> workers (N threads) -> writer (main thread, reordering)
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

    if !quiet {
        let info = parser.site_info();
        eprintln!("Database: {}", info.dbname);
        eprintln!("Using {} worker threads", num_threads);
    }

    // Bounded channel: parser -> workers (back-pressure to avoid unbounded memory growth)
    let (work_tx, work_rx) = std::sync::mpsc::sync_channel::<(u64, Page)>(num_threads * 2);
    let work_rx = Mutex::new(work_rx);

    // Unbounded channel: workers -> writer (results arrive out of order, reordered before writing)
    let (result_tx, result_rx) = std::sync::mpsc::channel::<(u64, AnalysisResult)>();

    // Track errors from the parser thread
    let parse_error: Mutex<Option<String>> = Mutex::new(None);

    std::thread::scope(|s| {
        // Spawn N worker threads that pull pages and analyze them
        for _ in 0..num_threads {
            let work_rx = &work_rx;
            let result_tx = result_tx.clone();
            s.spawn(move || {
                loop {
                    let item = work_rx.lock().unwrap().recv();
                    let (seq, page) = match item {
                        Ok(v) => v,
                        Err(_) => break, // channel closed, no more work
                    };
                    let result = match PageAnalysis::analyse_page(&page.revisions) {
                        Ok(analysis) => AnalysisResult::Ok(page, analysis),
                        Err(e) => AnalysisResult::Skipped(page.title.to_string(), e.to_string()),
                    };
                    // If the writer has dropped, stop
                    if result_tx.send((seq, result)).is_err() {
                        break;
                    }
                }
            });
        }

        // Drop the original result_tx so that result_rx closes once all workers finish
        drop(result_tx);

        // Parser thread: reads pages sequentially, applies namespace filter, sends to workers
        let parse_error_ref = &parse_error;
        s.spawn(move || {
            let mut seq = 0u64;
            loop {
                match parser.parse_page() {
                    Ok(Some(page)) => {
                        if !namespace_filter.is_empty()
                            && !namespace_filter.contains(&page.namespace)
                        {
                            continue;
                        }
                        if page_limit.map(|n| seq >= n).unwrap_or(false) {
                            break; // page limit reached
                        }
                        if work_tx.send((seq, page)).is_err() {
                            break; // workers gone
                        }
                        seq += 1;
                    }
                    Ok(None) => break, // end of stream
                    Err(e) => {
                        *parse_error_ref.lock().unwrap() = Some(format!("XML parse error: {e:?}"));
                        break;
                    }
                }
            }
            // work_tx is dropped here, signaling workers to finish
        });

        // Main thread: collect results, reorder, and write
        if format == Format::Json {
            write!(writer, "[").unwrap();
        }

        let mut next_seq: u64 = 0;
        let mut page_count: u64 = 0;
        let mut reorder_buf: BTreeMap<u64, AnalysisResult> = BTreeMap::new();

        for (seq, result) in &result_rx {
            reorder_buf.insert(seq, result);

            // Flush all consecutive ready results
            while let Some(result) = reorder_buf.remove(&next_seq) {
                next_seq += 1;
                match result {
                    AnalysisResult::Skipped(title, err) => {
                        if !quiet {
                            eprintln!("Warning: skipping '{title}': {err}");
                        }
                    }
                    AnalysisResult::Ok(page, analysis) => {
                        let page_title = page.title.clone();
                        write_page_result(&mut writer, page, &analysis, format, page_count).unwrap();
                        page_count += 1;
                        if !quiet && page_count % 100 == 0 {
                            eprintln!("Processed {page_count} pages (latest: {})", page_title);
                        }
                        drop(analysis);
                    }
                }
            }
        }

        if format == Format::Json {
            writeln!(writer, "]").unwrap();
        }

        writer.flush().unwrap();

        if !quiet {
            eprintln!("Done. Processed {page_count} pages.");
        }
    });

    // Check if the parser hit an error
    if let Some(err) = parse_error.into_inner().unwrap() {
        return Err(err.into());
    }

    Ok(())
}

fn write_page_result(
    writer: &mut Box<dyn Write>,
    page: Page,
    analysis: &PageAnalysis,
    format: Format,
    page_count: u64,
) -> Result<(), Box<dyn std::error::Error>> {
    match format {
        Format::Jsonl => {
            let obj = build_page_json(page, analysis);
            serde_json::to_writer(&mut *writer, &obj)?;
            writeln!(writer)?;
        }
        Format::Json => {
            if page_count > 0 {
                write!(writer, ",")?;
            }
            let obj = build_page_json(page, analysis);
            serde_json::to_writer(&mut *writer, &obj)?;
        }
        Format::Raw => {
            serde_json::to_writer(&mut *writer, analysis)?;
            writeln!(writer)?;
        }
    }
    Ok(())
}

fn build_page_json(page: Page, analysis: &PageAnalysis) -> serde_json::Value {
    let revisions_by_id: HashMap<i32, Revision> = page
        .revisions
        .into_iter()
        .map(|rev| (rev.id, rev))
        .collect();

    let revisions: Vec<serde_json::Value> = analysis
        .ordered_revisions
        .iter()
        .map(|rev_ptr| {
            let xml_revision = &revisions_by_id[&rev_ptr.id];
            serde_json::json!({
                "id": xml_revision.id,
                "timestamp": xml_revision.timestamp.to_rfc3339(),
                "editor": format_editor(&xml_revision.contributor),
            })
        })
        .collect();

    // Build all_tokens from the last (current) revision's content
    let last_rev = &analysis.current_revision;
    let tokens: Vec<serde_json::Value> = iterate_revision_tokens(analysis, last_rev)
        .map(|word_ptr| {
            let word_analysis = &analysis[word_ptr];
            let xml_origin_revision = &revisions_by_id[&word_analysis.origin_revision.id];
            serde_json::json!({
                "token_id": word_ptr.unique_id(),
                "str": &*word_ptr.value,
                "o_rev_id": xml_origin_revision.id,
                "editor": format_editor(&xml_origin_revision.contributor),
                "in": word_analysis.inbound.iter().map(|r| r.id).collect::<Vec<_>>(),
                "out": word_analysis.outbound.iter().map(|r| r.id).collect::<Vec<_>>(),
            })
        })
        .collect();

    serde_json::json!({
        "article_title": &*page.title,
        "namespace": page.namespace,
        "revisions": revisions,
        "spam_ids": &analysis.spam_ids,
        "all_tokens": tokens,
    })
}
