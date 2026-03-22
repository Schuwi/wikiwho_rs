// SPDX-License-Identifier: MPL-2.0
use std::io::{self, BufRead, BufReader, BufWriter, Write};
use std::process::ExitCode;

use wikiwho::algorithm::PageAnalysis;
use wikiwho::dump_parser::{Contributor, DumpParser};
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
  -n, --namespace NS      Only process pages in this namespace (repeatable)
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
    opts.optopt("o", "output", "Output file (\"-\" or omit for stdout)", "PATH");
    opts.optopt("f", "format", "Output format: jsonl (default), json, raw", "FORMAT");
    opts.optmulti("n", "namespace", "Only process pages in this namespace", "NS");
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

    let namespace_filter: Vec<i32> = matches
        .opt_strs("n")
        .iter()
        .map(|s| s.parse::<i32>())
        .collect::<Result<_, _>>()
        .map_err(|e| format!("invalid namespace: {e}"))?;

    let quiet = matches.opt_present("q");

    let input_path = matches.free.first().map(|s| s.as_str());
    let output_path = matches.opt_str("o");

    // Set up input reader with auto-decompression
    let reader: Box<dyn BufRead> = match input_path {
        None | Some("-") => Box::new(BufReader::new(io::stdin().lock())),
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
                    zstd::Encoder::new(file, 3)
                        .map_err(|e| format!("zstd init: {e}"))?,
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

    process(reader, writer, format, &namespace_filter, quiet)
}

fn process(
    reader: Box<dyn BufRead>,
    mut writer: Box<dyn Write>,
    format: Format,
    namespace_filter: &[i32],
    quiet: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut parser = DumpParser::new(reader).map_err(|e| format!("failed to initialize parser: {e:?}"))?;

    if !quiet {
        let info = parser.site_info();
        eprintln!("Database: {}", info.dbname);
    }

    let mut page_count: u64 = 0;

    if format == Format::Json {
        write!(writer, "[")?;
    }

    while let Some(page) = parser.parse_page().map_err(|e| format!("XML parse error: {e:?}"))? {
        if !namespace_filter.is_empty() && !namespace_filter.contains(&page.namespace) {
            continue;
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

        match format {
            Format::Jsonl => {
                let obj = build_page_json(&page, &analysis);
                serde_json::to_writer(&mut writer, &obj)?;
                writeln!(writer)?;
            }
            Format::Json => {
                if page_count > 0 {
                    write!(writer, ",")?;
                }
                let obj = build_page_json(&page, &analysis);
                serde_json::to_writer(&mut writer, &obj)?;
            }
            Format::Raw => {
                serde_json::to_writer(&mut writer, &analysis)?;
                writeln!(writer)?;
            }
        }

        page_count += 1;
        if !quiet && page_count % 100 == 0 {
            eprintln!("Processed {page_count} pages (latest: {})", page.title);
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

fn build_page_json(
    page: &wikiwho::dump_parser::Page,
    analysis: &PageAnalysis,
) -> serde_json::Value {
    let revisions: Vec<serde_json::Value> = analysis
        .ordered_revisions
        .iter()
        .map(|rev_ptr| {
            serde_json::json!({
                "id": rev_ptr.xml_revision.id,
                "timestamp": rev_ptr.xml_revision.timestamp.to_rfc3339(),
                "editor": format_editor(&rev_ptr.xml_revision.contributor),
            })
        })
        .collect();

    // Build all_tokens from the last (current) revision's content
    let last_rev = &analysis.current_revision;
    let tokens: Vec<serde_json::Value> = iterate_revision_tokens(analysis, last_rev)
        .map(|word_ptr| {
            let word_analysis = &analysis[word_ptr];
            serde_json::json!({
                "token_id": word_ptr.unique_id(),
                "str": &*word_ptr.value,
                "o_rev_id": word_analysis.origin_revision.xml_revision.id,
                "editor": format_editor(&word_analysis.origin_revision.xml_revision.contributor),
                "in": word_analysis.inbound.iter().map(|r| r.xml_revision.id).collect::<Vec<_>>(),
                "out": word_analysis.outbound.iter().map(|r| r.xml_revision.id).collect::<Vec<_>>(),
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
