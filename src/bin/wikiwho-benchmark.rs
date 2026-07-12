// SPDX-License-Identifier: MPL-2.0
//! Internal worker used by `scripts/wikiwho_bench.py`.

use std::fs::File;
use std::io::{self, BufRead, BufReader, BufWriter, Write};
use std::path::Path;
use std::time::{Duration, Instant};

use serde::Serialize;
use wikiwho::algorithm::PageAnalysis;
use wikiwho::dump_parser::{DumpParser, Page};

fn usage(program: &str) {
    eprintln!(
        "Usage:\n  {program} prepare --output CORPUS [--limit N] [--namespace NS]... INPUT...\n  {program} decompress --output XML INPUT\n  {program} run --corpus CORPUS\n  {program} run-xml --mode parse|end-to-end [--limit N] [--namespace NS]... INPUT...\n\nThis is an internal worker. Prefer scripts/wikiwho_bench.py for normal use."
    );
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    match args.get(1).map(String::as_str) {
        Some("prepare") => prepare(&args[2..]),
        Some("decompress") => decompress(&args[2..]),
        Some("run") => run_corpus(&args[2..]),
        Some("run-xml") => run_xml(&args[2..]),
        _ => {
            usage(&args[0]);
            Err("missing or unknown subcommand".into())
        }
    }
}

fn decompress(args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    let mut opts = getopts::Options::new();
    opts.reqopt("o", "output", "uncompressed XML to create", "PATH");
    let matches = opts.parse(args)?;
    if matches.free.len() != 1 {
        return Err("decompress requires exactly one input".into());
    }
    let output = matches.opt_str("output").expect("required by getopts");
    let mut reader = input_reader(&matches.free[0])?;
    let mut writer = BufWriter::new(File::create(&output)?);
    io::copy(&mut reader, &mut writer)?;
    writer.flush()?;
    Ok(())
}

fn input_reader(path: &str) -> Result<Box<dyn BufRead>, Box<dyn std::error::Error>> {
    let file = File::open(path).map_err(|e| format!("cannot open input '{path}': {e}"))?;
    let reader: Box<dyn BufRead> = if path.ends_with(".bz2") {
        Box::new(BufReader::new(bzip2::read::BzDecoder::new(file)))
    } else if path.ends_with(".gz") {
        Box::new(BufReader::new(flate2::read::GzDecoder::new(file)))
    } else if path.ends_with(".zst") || path.ends_with(".zstd") {
        Box::new(BufReader::new(
            zstd::Decoder::new(file).map_err(|e| format!("cannot decode '{path}': {e}"))?,
        ))
    } else {
        Box::new(BufReader::new(file))
    };
    Ok(reader)
}

fn prepare(args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    let mut opts = getopts::Options::new();
    opts.reqopt("o", "output", "JSONL corpus to create", "PATH");
    opts.optopt("N", "limit", "maximum number of pages in total", "N");
    opts.optmulti("n", "namespace", "only include this namespace", "NS");
    let matches = opts.parse(args)?;
    if matches.free.is_empty() {
        return Err("prepare requires at least one input dump".into());
    }

    let limit: Option<usize> = matches.opt_get("limit")?;
    if limit == Some(0) {
        return Err("--limit must be at least 1".into());
    }
    let namespaces: Vec<i32> = matches
        .opt_strs("namespace")
        .iter()
        .map(|value| value.parse())
        .collect::<Result<_, _>>()?;
    let output = matches.opt_str("output").expect("required by getopts");
    let mut writer = BufWriter::new(
        File::create(&output).map_err(|e| format!("cannot create corpus '{output}': {e}"))?,
    );

    let mut pages = 0usize;
    let mut revisions = 0usize;
    let mut text_bytes = 0usize;
    'inputs: for input in &matches.free {
        let mut parser = DumpParser::new(input_reader(input)?)
            .map_err(|e| format!("cannot parse site info in '{input}': {e}"))?;
        while let Some(page) = parser
            .parse_page()
            .map_err(|e| format!("cannot parse page from '{input}': {e}"))?
        {
            if !namespaces.is_empty() && !namespaces.contains(&page.namespace) {
                continue;
            }
            revisions += page.revisions.len();
            text_bytes += page
                .revisions
                .iter()
                .map(|revision| revision.text.len())
                .sum::<usize>();
            serde_json::to_writer(&mut writer, &page)?;
            writer.write_all(b"\n")?;
            pages += 1;
            if limit.is_some_and(|limit| pages >= limit) {
                break 'inputs;
            }
        }
    }
    writer.flush()?;
    if pages == 0 {
        return Err("no pages matched the requested inputs and filters".into());
    }

    serde_json::to_writer(
        io::stdout().lock(),
        &CorpusSummary {
            pages,
            revisions,
            text_bytes,
        },
    )?;
    println!();
    Ok(())
}

#[derive(Serialize)]
struct CorpusSummary {
    pages: usize,
    revisions: usize,
    text_bytes: usize,
}

#[derive(Serialize)]
struct RunReport<'a> {
    implementation: &'a str,
    mode: &'a str,
    pages: usize,
    revisions: usize,
    text_bytes: usize,
    analysed_revisions: usize,
    tokens: usize,
    seconds: f64,
    checksum: u64,
}

fn run_corpus(args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    let mut opts = getopts::Options::new();
    opts.reqopt("c", "corpus", "prepared JSONL corpus", "PATH");
    let matches = opts.parse(args)?;
    if !matches.free.is_empty() {
        return Err(format!("unexpected argument: {}", matches.free[0]).into());
    }
    let corpus = matches.opt_str("corpus").expect("required by getopts");
    let reader = BufReader::new(
        File::open(Path::new(&corpus))
            .map_err(|e| format!("cannot open corpus '{corpus}': {e}"))?,
    );

    let mut pages = 0usize;
    let mut revisions = 0usize;
    let mut text_bytes = 0usize;
    let mut analysed_revisions = 0usize;
    let mut tokens = 0usize;
    let mut elapsed = Duration::ZERO;
    let mut checksum = 0u64;
    for (index, line) in reader.lines().enumerate() {
        let line = line.map_err(|e| format!("cannot read corpus line {}: {e}", index + 1))?;
        let page: Page = serde_json::from_str(&line)
            .map_err(|e| format!("invalid corpus line {}: {e}", index + 1))?;
        pages += 1;
        revisions += page.revisions.len();
        text_bytes += page
            .revisions
            .iter()
            .map(|revision| revision.text.len())
            .sum::<usize>();

        let start = Instant::now();
        let analysis = PageAnalysis::analyse_page(std::hint::black_box(&page.revisions))
            .map_err(|e| format!("cannot analyse page {:?}: {e}", page.title))?;
        elapsed += start.elapsed();

        analysed_revisions += analysis.ordered_revisions.len();
        tokens += analysis.words.len();
        checksum = checksum
            .wrapping_mul(1_099_511_628_211)
            .wrapping_add(analysis.words.len() as u64)
            .wrapping_add((analysis.ordered_revisions.len() as u64) << 32);
        std::hint::black_box(&analysis);
    }

    serde_json::to_writer(
        io::stdout().lock(),
        &RunReport {
            implementation: "rust",
            mode: "algorithm",
            pages,
            revisions,
            text_bytes,
            analysed_revisions,
            tokens,
            seconds: elapsed.as_secs_f64(),
            checksum,
        },
    )?;
    println!();
    Ok(())
}

fn run_xml(args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    let mut opts = getopts::Options::new();
    opts.reqopt("m", "mode", "parse or end-to-end", "MODE");
    opts.optopt("N", "limit", "maximum number of pages in total", "N");
    opts.optmulti("n", "namespace", "only include this namespace", "NS");
    let matches = opts.parse(args)?;
    if matches.free.is_empty() {
        return Err("run-xml requires at least one input".into());
    }
    let mode = matches.opt_str("mode").expect("required by getopts");
    if mode != "parse" && mode != "end-to-end" {
        return Err(format!("unknown XML benchmark mode: {mode}").into());
    }
    let limit: Option<usize> = matches.opt_get("limit")?;
    let namespaces: Vec<i32> = matches
        .opt_strs("namespace")
        .iter()
        .map(|value| value.parse())
        .collect::<Result<_, _>>()?;

    let mut pages = 0usize;
    let mut revisions = 0usize;
    let mut text_bytes = 0usize;
    let mut analysed_revisions = 0usize;
    let mut tokens = 0usize;
    let mut checksum = 0u64;
    let start = Instant::now();
    'inputs: for input in &matches.free {
        let mut parser = DumpParser::new(input_reader(input)?)?;
        while let Some(page) = parser.parse_page()? {
            if !namespaces.is_empty() && !namespaces.contains(&page.namespace) {
                continue;
            }
            pages += 1;
            revisions += page.revisions.len();
            text_bytes += page
                .revisions
                .iter()
                .map(|revision| revision.text.len())
                .sum::<usize>();
            if mode == "end-to-end" {
                let analysis = PageAnalysis::analyse_page(std::hint::black_box(&page.revisions))
                    .map_err(|e| format!("cannot analyse page {:?}: {e}", page.title))?;
                analysed_revisions += analysis.ordered_revisions.len();
                tokens += analysis.words.len();
                checksum = checksum
                    .wrapping_mul(1_099_511_628_211)
                    .wrapping_add(analysis.words.len() as u64)
                    .wrapping_add((analysis.ordered_revisions.len() as u64) << 32);
                std::hint::black_box(&analysis);
            } else {
                checksum = checksum
                    .wrapping_mul(1_099_511_628_211)
                    .wrapping_add(page.revisions.len() as u64)
                    .wrapping_add(page.title.len() as u64);
                std::hint::black_box(&page);
            }
            if limit.is_some_and(|limit| pages >= limit) {
                break 'inputs;
            }
        }
    }
    let elapsed = start.elapsed();
    if pages == 0 {
        return Err("no pages matched the requested inputs and filters".into());
    }

    serde_json::to_writer(
        io::stdout().lock(),
        &RunReport {
            implementation: "rust",
            mode: &mode,
            pages,
            revisions,
            text_bytes,
            analysed_revisions,
            tokens,
            seconds: elapsed.as_secs_f64(),
            checksum,
        },
    )?;
    println!();
    Ok(())
}
