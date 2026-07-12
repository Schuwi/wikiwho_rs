// SPDX-License-Identifier: MPL-2.0
//! Internal worker used by `scripts/wikiwho_bench.py`.

use std::fs::File;
use std::io::{self, BufRead, BufReader, BufWriter, Write};
use std::path::Path;
use std::time::{Duration, Instant};

use serde::Serialize;
use wikiwho::algorithm::PageAnalysis;
use wikiwho::dump_parser::{Contributor, DumpParser, Namespace, Page, Revision, SiteInfo, Text};

fn usage(program: &str) {
    eprintln!(
        "Usage:\n  {program} prepare --output CORPUS --xml-output XML [--limit N] [--namespace NS]... INPUT...\n  {program} run --corpus CORPUS\n  {program} run-xml --mode parse|end-to-end [--limit N] [--namespace NS]... INPUT...\n\nThis is an internal worker. Prefer scripts/wikiwho_bench.py for normal use."
    );
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    match args.get(1).map(String::as_str) {
        Some("prepare") => prepare(&args[2..]),
        Some("run") => run_corpus(&args[2..]),
        Some("run-xml") => run_xml(&args[2..]),
        _ => {
            usage(&args[0]);
            Err("missing or unknown subcommand".into())
        }
    }
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
    opts.reqopt("", "xml-output", "bounded XML corpus to create", "PATH");
    opts.optopt("N", "limit", "maximum number of pages in total", "N");
    opts.optopt("", "stride", "select every Nth matching page", "N");
    opts.optmulti("n", "namespace", "only include this namespace", "NS");
    let matches = opts.parse(args)?;
    if matches.free.is_empty() {
        return Err("prepare requires at least one input dump".into());
    }

    let limit: Option<usize> = matches.opt_get("limit")?;
    if limit == Some(0) {
        return Err("--limit must be at least 1".into());
    }
    let stride: usize = matches.opt_get("stride")?.unwrap_or(1);
    if stride == 0 {
        return Err("--stride must be at least 1".into());
    }
    let namespaces: Vec<i32> = matches
        .opt_strs("namespace")
        .iter()
        .map(|value| value.parse())
        .collect::<Result<_, _>>()?;
    let output = matches.opt_str("output").expect("required by getopts");
    let xml_output = matches.opt_str("xml-output").expect("required by getopts");
    let mut writer = BufWriter::new(
        File::create(&output).map_err(|e| format!("cannot create corpus '{output}': {e}"))?,
    );
    let mut xml_writer = BufWriter::new(
        File::create(&xml_output)
            .map_err(|e| format!("cannot create XML corpus '{xml_output}': {e}"))?,
    );

    let mut pages = 0usize;
    let mut matching_pages_seen = 0usize;
    let mut revisions = 0usize;
    let mut text_bytes = 0usize;
    let mut header_written = false;
    'inputs: for input in &matches.free {
        let mut parser = DumpParser::new(input_reader(input)?)
            .map_err(|e| format!("cannot parse site info in '{input}': {e}"))?;
        if !header_written {
            write_xml_header(&mut xml_writer, parser.site_info())?;
            header_written = true;
        }
        while let Some(page) = parser
            .parse_page()
            .map_err(|e| format!("cannot parse page from '{input}': {e}"))?
        {
            if !namespaces.is_empty() && !namespaces.contains(&page.namespace) {
                continue;
            }
            matching_pages_seen += 1;
            if !(matching_pages_seen - 1).is_multiple_of(stride) {
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
            write_xml_page(&mut xml_writer, &page, pages + 1)?;
            pages += 1;
            if limit.is_some_and(|limit| pages >= limit) {
                break 'inputs;
            }
        }
    }
    xml_writer.write_all(b"</mediawiki>\n")?;
    writer.flush()?;
    xml_writer.flush()?;
    if pages == 0 {
        return Err("no pages matched the requested inputs and filters".into());
    }

    serde_json::to_writer(
        io::stdout().lock(),
        &CorpusSummary {
            pages,
            revisions,
            text_bytes,
            matching_pages_seen,
            json_bytes: std::fs::metadata(&output)?.len(),
            xml_bytes: std::fs::metadata(&xml_output)?.len(),
        },
    )?;
    println!();
    Ok(())
}

fn write_escaped(writer: &mut impl Write, value: &str) -> io::Result<()> {
    writer.write_all(quick_xml::escape::escape(value).as_bytes())
}

fn write_xml_header(writer: &mut impl Write, site_info: &SiteInfo) -> io::Result<()> {
    writer.write_all(
        b"<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<mediawiki xmlns=\"http://www.mediawiki.org/xml/export-0.11/\" version=\"0.11\" xml:lang=\"en\">\n<siteinfo><sitename>WikiWho benchmark corpus</sitename><dbname>",
    )?;
    write_escaped(writer, &site_info.dbname)?;
    writer.write_all(b"</dbname><base>https://example.invalid/wiki/Main_Page</base><generator>wikiwho-benchmark</generator><case>first-letter</case><namespaces>")?;
    let mut namespaces: Vec<_> = site_info.namespaces.iter().collect();
    namespaces.sort_unstable_by_key(|(key, _)| **key);
    for (key, namespace) in namespaces {
        write!(writer, "<namespace key=\"{key}\" case=\"first-letter\">")?;
        if let Namespace::Named(name) = namespace {
            write_escaped(writer, name)?;
        }
        writer.write_all(b"</namespace>")?;
    }
    writer.write_all(b"</namespaces></siteinfo>\n")
}

fn write_contributor(writer: &mut impl Write, contributor: &Contributor) -> io::Result<()> {
    if contributor.username.is_empty() {
        return writer.write_all(b"<contributor deleted=\"deleted\" />");
    }
    writer.write_all(b"<contributor>")?;
    if let Some(id) = contributor.id {
        writer.write_all(b"<username>")?;
        write_escaped(writer, &contributor.username)?;
        write!(writer, "</username><id>{id}</id>")?;
    } else {
        writer.write_all(b"<ip>")?;
        write_escaped(writer, &contributor.username)?;
        writer.write_all(b"</ip>")?;
    }
    writer.write_all(b"</contributor>")
}

fn write_xml_revision(writer: &mut impl Write, revision: &Revision) -> io::Result<()> {
    write!(
        writer,
        "<revision><id>{}</id><timestamp>{}</timestamp>",
        revision.id,
        revision
            .timestamp
            .to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
    )?;
    write_contributor(writer, &revision.contributor)?;
    if let Some(comment) = &revision.comment {
        writer.write_all(b"<comment>")?;
        write_escaped(writer, comment)?;
        writer.write_all(b"</comment>")?;
    }
    if revision.minor {
        writer.write_all(b"<minor />")?;
    }
    match &revision.text {
        Text::Normal(text) => {
            if text.is_empty() {
                writer.write_all(b"<text bytes=\"0\" xml:space=\"preserve\" />")?;
            } else {
                write!(
                    writer,
                    "<text bytes=\"{}\" xml:space=\"preserve\">",
                    text.len()
                )?;
                write_escaped(writer, text)?;
                writer.write_all(b"</text>")?;
            }
        }
        Text::Deleted => writer.write_all(b"<text deleted=\"deleted\" />")?,
    }
    if let Some(sha1) = revision.sha1 {
        writer.write_all(b"<sha1>")?;
        writer.write_all(&sha1.0)?;
        writer.write_all(b"</sha1>")?;
    }
    writer.write_all(b"</revision>")
}

fn write_xml_page(writer: &mut impl Write, page: &Page, page_id: usize) -> io::Result<()> {
    writer.write_all(b"<page><title>")?;
    write_escaped(writer, &page.title)?;
    write!(
        writer,
        "</title><ns>{}</ns><id>{page_id}</id>",
        page.namespace
    )?;
    for revision in &page.revisions {
        write_xml_revision(writer, revision)?;
    }
    writer.write_all(b"</page>\n")
}

#[derive(Serialize)]
struct CorpusSummary {
    pages: usize,
    revisions: usize,
    text_bytes: usize,
    matching_pages_seen: usize,
    json_bytes: u64,
    xml_bytes: u64,
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

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::io::Cursor;

    use super::*;

    #[test]
    fn canonical_xml_preserves_empty_normal_revision_text() {
        let site_info = SiteInfo {
            dbname: "testwiki".into(),
            namespaces: HashMap::from([(0, Namespace::Default)]),
        };
        let page = Page {
            title: "Cleared page".into(),
            namespace: 0,
            revisions: vec![Revision {
                id: 42,
                timestamp: chrono::DateTime::from_timestamp(1_700_000_000, 0).unwrap(),
                contributor: Contributor {
                    username: "127.0.0.1".into(),
                    id: None,
                },
                text: Text::Normal(String::new()),
                sha1: None,
                comment: Some("page cleared".into()),
                minor: false,
            }],
        };

        let mut xml = Vec::new();
        write_xml_header(&mut xml, &site_info).unwrap();
        write_xml_page(&mut xml, &page, 1).unwrap();
        xml.extend_from_slice(b"</mediawiki>\n");

        let mut parser = DumpParser::new(Cursor::new(xml)).unwrap();
        let parsed = parser.parse_page().unwrap().unwrap();
        assert_eq!(parsed.revisions.len(), 1);
        assert_eq!(parsed.revisions[0].text, Text::Normal(String::new()));
    }
}
