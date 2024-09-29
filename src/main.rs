use algorithm::Analysis;
use clap::Parser;
use dump_parser::DumpParser;
use std::collections::HashMap;
use std::fs::File;
use std::io::BufReader;
use std::path::PathBuf;

mod algorithm;
mod dump_parser;
mod utils;

#[derive(Debug, clap::Parser)]
struct CommandLine {
    input_file: PathBuf,
}

fn main() {
    let args: CommandLine = CommandLine::parse();

    let file = File::open(args.input_file).expect("file not found");
    let reader = BufReader::new(file);
    let reader = zstd::stream::Decoder::with_buffer(reader).unwrap();
    let reader = BufReader::new(reader);

    let mut parser = DumpParser::new(reader).expect("Failed to create parser");
    println!("Site info: {:?}", parser.site_info());

    while let Some(page) = parser.parse_page().expect("Failed to parse page") {
        if page.namespace != 0 {
            continue;
        }

        let (mut analysis, analysis_result) = Analysis::analyse_page(&page.title, &page.revisions).expect("Failed to analyse page");
        let latest_rev_id = analysis_result.ordered_revisions.last().unwrap();
        let latest_rev_pointer = analysis_result.revisions[latest_rev_id].clone();

        println!("Page: {}", page.title);
        println!("Latest revision: {}", latest_rev_id);

        let mut author_contributions = HashMap::new();
        analysis.iterate_words_in_revisions(&[latest_rev_pointer], |word| {
            let origin_rev_id = word.origin_rev_id;
            let origin_rev = page.revisions.iter().find(|rev| rev.id == origin_rev_id).unwrap();
            // let origin_rev = &analysis_result.revisions[&origin_rev_id];

            let author = origin_rev.contributor.clone();
            // let author = origin_rev.editor.clone();
            let author_contribution = author_contributions.entry(author).or_insert(0);
            *author_contribution += 1;
        });

        // Find top 5 authors and everyone with at least 5% of the total contributions or at least 25 tokens
        let total_tokens = author_contributions.values().sum::<usize>();
        let mut author_contributions: Vec<_> = author_contributions.into_iter().collect();
        author_contributions.sort_by_key(|(_, count)| *count);
        author_contributions.reverse();

        let mut top_authors = Vec::new();
        let mut other_authors = Vec::new();
        for (author, count) in author_contributions {
            if count >= 25 || count as f64 / total_tokens as f64 >= 0.05 {
                top_authors.push((author, count));
            } else {
                other_authors.push((author, count));
            }
        }

        println!("Top authors:");
        for (author, count) in top_authors {
            println!("{:?}: {}", author, count);
        }
    }
}
