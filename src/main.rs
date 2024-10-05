use algorithm::Analysis;
use clap::Parser;
use dump_parser::{Contributor, DumpParser};
use json_writer::JSONObjectWriter;
use std::collections::HashMap;
use std::fs::File;
use std::io::BufReader;
use std::path::PathBuf;

mod algorithm;
mod dump_parser;
// it only makes sense to compare the algorithm to python if the same diff algorithm is used
#[cfg(all(test, feature = "python-diff"))]
mod integration_tests;
#[cfg(test)]
mod test_support;
mod utils;

#[derive(Debug, clap::Parser)]
struct CommandLine {
    input_file: PathBuf,
}

fn main() {
    let args: CommandLine = CommandLine::parse();

    let file = File::open(&args.input_file)
        .unwrap_or_else(|_| panic!("file not found: {}", args.input_file.display()));
    let reader = BufReader::new(file);
    let reader = zstd::stream::Decoder::with_buffer(reader).unwrap();
    let reader = BufReader::new(reader);

    let mut parser = DumpParser::new(reader).expect("Failed to create parser");
    eprintln!("Site info: {:?}", parser.site_info());

    let mut output = String::new();
    while let Some(page) = parser.parse_page().expect("Failed to parse page") {
        // if page.namespace != 0 {
        //     continue;
        // }

        let (analysis, analysis_result) =
            Analysis::analyse_page(&page.revisions).expect("Failed to analyse page");
        let latest_rev_id = *analysis_result.ordered_revisions.last().unwrap();
        let latest_rev_pointer = analysis_result.revisions[&latest_rev_id].clone();

        let mut author_contributions = HashMap::new();
        for word_pointer in utils::iterate_revision_tokens(&analysis, &latest_rev_pointer) {
            let origin_rev_id = analysis[word_pointer].origin_rev_id;
            let origin_rev = &analysis_result.revisions[&origin_rev_id];

            let author = origin_rev.xml_revision.contributor.clone();
            let author_contribution = author_contributions.entry(author).or_insert(0);
            *author_contribution += 1;
        }

        // Find top 5 authors and everyone with at least 5% of the total contributions or at least 25 tokens
        /*
        total_contributions = sum(author_contributions.values())
        top_authors = sorted(author_contributions.items(), key=lambda x: x[1], reverse=True)[:5]
        top_authors += filter(lambda x: (x[1] / total_contributions >= 0.05 or x[1] >= 25) and not (x in top_authors), author_contributions.items())
         */
        let total_contributions: usize = author_contributions.values().sum();
        let mut top_authors: Vec<(&Contributor, &usize)> = author_contributions.iter().collect();
        top_authors.sort_by(|a, b| b.1.cmp(a.1).then_with(|| b.0.username.cmp(&a.0.username))); /* note reversed order on name comparison to match python script */
        top_authors.truncate(5);
        top_authors.extend(author_contributions.iter().filter(|(_, count)| {
            **count as f64 / total_contributions as f64 >= 0.05 || **count >= 25
        }));
        top_authors.sort_by(|a, b| {
            a.0.id
                .cmp(&b.0.id)
                .then_with(|| a.0.username.cmp(&b.0.username))
        });
        top_authors.dedup();
        top_authors.sort_by(|a, b| b.1.cmp(a.1).then_with(|| b.0.username.cmp(&a.0.username)));

        let mut object_writer = JSONObjectWriter::new(&mut output);

        object_writer.value("page", page.title.as_str());
        object_writer.value("ns", page.namespace);
        let mut array_writer = object_writer.array("top_authors");
        for (author, count) in top_authors {
            let mut author_writer = array_writer.object();
            author_writer.value("id", author.id);
            author_writer.value("text", author.username.as_str());
            author_writer.value("contributions", *count as u64);
        }
        array_writer.end();
        object_writer.value("total_tokens", total_contributions as u64);

        // let mut array_writer = object_writer.array("current_tokens");
        // for word in utils::iterate_revision_tokens(&analysis, &latest_rev_pointer) {
        //     array_writer.value(word.value.as_str());
        // }
        // array_writer.end();

        object_writer.end();

        println!("{output}");
        output.clear();
    }
}

/*
DEBUGGING TODO:
## "Nodb" page [x]
expected: {"page":"Nodb","top_authors":[{"id":128144,"text":"Nerd","contributions":24},{"id":1390,"text":"Pajz","contributions":22},{"id":306,"text":"Melancholie","contributions":2}],"total_tokens":48}
got: {"page":"Nodb","top_authors":[{"id":128144,"text":"Nerd","contributions":31},{"id":1390,"text":"Pajz","contributions":17}],"total_tokens":48}
-> manual revision of input file: Melancholie has 2 contributions to the page,
                                  contributions of Nerd are closer to 24 than 31,
                                  contributions of Pajz are closer to 22 than 17
=> highly depends on the diff algorithm - using LCS insted of Myers gets a closer result to the original implementation

## Approximate comparison of results with the original implementation [ ]
 */
