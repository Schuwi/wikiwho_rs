// SPDX-License-Identifier: MPL-2.0
//! Parser integration tests.
//!
use std::io::Cursor;

#[cfg(any(feature = "strict", feature = "python-diff"))]
use std::{fs::File, io::BufReader};

#[cfg(feature = "python-diff")]
use pyo3::{prelude::*, types::PyBytes};
use wikiwho::dump_parser::{DumpParser, Namespace, Text};

#[cfg(feature = "strict")]
const DEFAULT_REFERENCE_DUMP: &str =
    "dev-data/reference-dumps/dewiktionary-20240901-pages-meta-history.xml.zst";
#[cfg(feature = "python-diff")]
const DEFAULT_COMPARISON_DUMP: &str =
    "dev-data/reference-dumps/dewiktionary-20240901-ci-subset.xml.zst";

/// A revision whose wikitext contains entity references (`&lt;`, `&gt;`).
/// quick-xml splits character data into multiple `Text` events at entity
/// boundaries and emits each entity as its own `GeneralRef` event. The parser
/// must accumulate every chunk and resolve the entities; the prior code
/// overwrote on each `Text` event, keeping only the run after the last entity
/// and dropping the entity characters — ~99% text loss on real articles.
const DUMP_WITH_ENTITIES: &str = r#"<mediawiki xmlns="http://www.mediawiki.org/xml/export-0.11/" version="0.11" xml:lang="en">
	<siteinfo><sitename>W</sitename><namespaces><namespace key="0" case="first-letter"/></namespaces></siteinfo>
	<page><title>T</title><ns>0</ns><id>1</id>
		<revision><id>1</id><timestamp>2020-01-01T00:00:00Z</timestamp>
			<contributor><username>A</username></contributor>
			<comment>before &amp; after</comment>
			<text xml:space="preserve" sha1="1234567890123456789012345678901">AAA words here. &lt;ref&gt;x&lt;/ref&gt; BBB words here.</text>
			<sha1>1234567890123456789012345678901</sha1>
		</revision>
		<revision><id>2</id><timestamp>2020-01-02T00:00:00Z</timestamp>
			<contributor deleted="deleted" />
			<text deleted="deleted" />
		</revision>
	</page>
</mediawiki>"#;

const DUMP_WITH_BOTH_CONTRIBUTOR_FIELDS: &str = r#"<mediawiki>
	<siteinfo><namespaces><namespace key="0" /></namespaces></siteinfo>
	<page><title>T</title><ns>0</ns><id>1</id>
		<revision><id>1</id><timestamp>2020-01-01T00:00:00Z</timestamp>
			<contributor><ip>127.0.0.1</ip><username>Alice</username></contributor><text>x</text>
		</revision>
		<revision><id>2</id><timestamp>2020-01-02T00:00:00Z</timestamp>
			<contributor><username>Bob</username><ip>127.0.0.2</ip></contributor><text>y</text>
		</revision>
	</page>
</mediawiki>"#;

const DUMP_WITH_CONFLICTING_SHA1_FIELDS: &str = r#"<mediawiki>
	<siteinfo><namespaces><namespace key="0" /></namespaces></siteinfo>
	<page><title>T</title><ns>0</ns><id>1</id>
		<revision><id>1</id><timestamp>2020-01-01T00:00:00Z</timestamp>
			<contributor><username>Alice</username></contributor>
			<text sha1="aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa">x</text>
			<sha1>bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb</sha1>
		</revision>
	</page>
</mediawiki>"#;

const DUMP_WITH_CRLF: &str = concat!(
    "<mediawiki><siteinfo><namespaces><namespace key=\"0\" /></namespaces></siteinfo>",
    "<page><title>T</title><ns>0</ns><id>1</id><revision><id>1</id>",
    "<timestamp>2020-01-01T00:00:00Z</timestamp><contributor><username>A</username></contributor>",
    "<text>first line\r\nsecond line\rthird line</text></revision></page></mediawiki>",
);

#[test]
fn revision_text_survives_entity_references() {
    let mut parser =
        DumpParser::new(Cursor::new(DUMP_WITH_ENTITIES.as_bytes())).expect("parser init");
    let page = parser
        .parse_page()
        .expect("parse_page")
        .expect("one page present");
    let text = page.revisions[0].text.as_str();

    // Text before the first entity used to be dropped by the overwrite bug.
    assert!(
        text.contains("AAA words here."),
        "leading text lost: {text:?}"
    );
    // Text after the last entity (all that used to survive).
    assert!(
        text.contains("BBB words here."),
        "trailing text lost: {text:?}"
    );
    // The entity characters themselves must be resolved, not discarded.
    assert!(
        text.contains("<ref>"),
        "entity chars lost / markup merged: {text:?}"
    );
    assert_eq!(
        page.revisions[0].comment.as_deref(),
        Some("before & after"),
        "comment chunks and entity must all survive"
    );
    assert_eq!(
        page.revisions[0].sha1.as_ref().map(|sha1| &sha1.0),
        Some(b"1234567890123456789012345678901"),
        "SHA-1 from the text attribute must be retained"
    );
    assert!(page.revisions[1].contributor.username.is_empty());
    assert_eq!(page.revisions[1].contributor.id, None);
    assert_eq!(page.revisions[1].text, Text::Deleted);
}

#[test]
fn xml_1_0_line_endings_are_normalized() {
    let mut parser = DumpParser::new(Cursor::new(DUMP_WITH_CRLF)).unwrap();
    let page = parser.parse_page().unwrap().unwrap();

    assert_eq!(
        page.revisions[0].text.as_str(),
        "first line\nsecond line\nthird line"
    );
}

#[test]
fn xml_1_0_attribute_references_are_normalized() {
    let xml = concat!(
        r#"<?xml version="1.0" encoding="UTF-8"?>"#,
        r#"<mediawiki><siteinfo><dbname>testwiki</dbname><namespaces>"#,
        r#"<namespace key="&#48;">Main</namespace>"#,
        r#"</namespaces></siteinfo></mediawiki>"#,
    );

    let parser = DumpParser::new(Cursor::new(xml)).unwrap();

    assert_eq!(
        parser.site_info().namespaces.get(&0),
        Some(&Namespace::Named("Main".into()))
    );
}

#[cfg(not(feature = "strict"))]
#[test]
fn contributor_username_takes_precedence_over_ip() {
    let mut parser = DumpParser::new(Cursor::new(DUMP_WITH_BOTH_CONTRIBUTOR_FIELDS)).unwrap();
    let page = parser.parse_page().unwrap().unwrap();

    assert_eq!(page.revisions[0].contributor.username, "Alice");
    assert_eq!(page.revisions[1].contributor.username, "Bob");

    #[cfg(feature = "python-diff")]
    assert_parsers_match(DUMP_WITH_BOTH_CONTRIBUTOR_FIELDS.as_bytes()).unwrap();
}

#[cfg(feature = "strict")]
#[test]
fn strict_mode_rejects_username_and_ip_together() {
    let mut parser = DumpParser::new(Cursor::new(DUMP_WITH_BOTH_CONTRIBUTOR_FIELDS)).unwrap();
    assert!(matches!(
        parser.parse_page(),
        Err(wikiwho::dump_parser::ParsingError::ConflictingContributorIdentity)
    ));
}

#[cfg(not(feature = "strict"))]
#[test]
fn text_sha1_takes_precedence_over_conflicting_element() {
    let mut parser = DumpParser::new(Cursor::new(DUMP_WITH_CONFLICTING_SHA1_FIELDS)).unwrap();
    let page = parser.parse_page().unwrap().unwrap();

    assert_eq!(
        page.revisions[0].sha1.as_ref().map(|sha1| &sha1.0),
        Some(b"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa")
    );

    #[cfg(feature = "python-diff")]
    assert_parsers_match(DUMP_WITH_CONFLICTING_SHA1_FIELDS.as_bytes()).unwrap();
}

#[cfg(feature = "strict")]
#[test]
fn strict_mode_rejects_conflicting_sha1_values() {
    let mut parser = DumpParser::new(Cursor::new(DUMP_WITH_CONFLICTING_SHA1_FIELDS)).unwrap();
    assert!(matches!(
        parser.parse_page(),
        Err(wikiwho::dump_parser::ParsingError::ConflictingSha1Values)
    ));
}

#[cfg(feature = "python-diff")]
fn assert_parsers_match(xml: &[u8]) -> PyResult<()> {
    let mut rust_parser = DumpParser::new(Cursor::new(xml)).expect("Rust parser init");

    Python::attach(|py| {
        let bytes_io = PyModule::import(py, "io")?
            .getattr("BytesIO")?
            .call1((PyBytes::new(py, xml),))?;
        let python_dump = PyModule::import(py, "mwxml")?
            .getattr("Dump")?
            .call_method1("from_file", (bytes_io,))?;

        for (page_index, python_page) in python_dump.try_iter()?.enumerate() {
            let python_page = python_page?;
            let rust_page = rust_parser
                .parse_page()
                .unwrap_or_else(|err| panic!("Rust parser failed at page {page_index}: {err:?}"))
                .unwrap_or_else(|| panic!("Rust parser ended before Python at page {page_index}"));

            let python_title: String = python_page.getattr("title")?.extract()?;
            let python_namespace: i32 = python_page.getattr("namespace")?.extract()?;
            assert_eq!(rust_page.title.as_str(), python_title, "page {page_index}");
            assert_eq!(
                rust_page.namespace, python_namespace,
                "namespace of page {page_index} ({python_title:?})"
            );

            let mut python_revision_count = 0;
            for (revision_index, python_revision) in python_page.try_iter()?.enumerate() {
                let python_revision = python_revision?;
                let rust_revision = rust_page.revisions.get(revision_index).unwrap_or_else(|| {
                    panic!(
                        "Rust parser returned fewer revisions than Python for page {page_index} ({python_title:?})"
                    )
                });
                python_revision_count += 1;

                let context =
                    format!("revision {revision_index} of page {page_index} ({python_title:?})");
                assert_eq!(
                    rust_revision.id,
                    python_revision.getattr("id")?.extract::<i32>()?,
                    "id of {context}"
                );
                assert_eq!(
                    rust_revision
                        .timestamp
                        .to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
                    python_revision.getattr("timestamp")?.str()?.to_str()?,
                    "timestamp of {context}"
                );

                let python_user = python_revision.getattr("user")?;
                if python_user.is_none() {
                    assert!(
                        rust_revision.contributor.username.is_empty(),
                        "deleted contributor name of {context}"
                    );
                    assert_eq!(
                        rust_revision.contributor.id, None,
                        "deleted contributor id of {context}"
                    );
                } else {
                    assert_eq!(
                        rust_revision.contributor.username.as_str(),
                        python_user.getattr("text")?.extract::<String>()?,
                        "contributor name of {context}"
                    );
                    assert_eq!(
                        rust_revision.contributor.id,
                        python_user.getattr("id")?.extract::<Option<i32>>()?,
                        "contributor id of {context}"
                    );
                }

                let python_text_deleted: bool = python_revision
                    .getattr("deleted")?
                    .getattr("text")?
                    .extract()?;
                let python_text: Option<String> = python_revision.getattr("text")?.extract()?;
                let expected_text = if python_text_deleted {
                    Text::Deleted
                } else {
                    Text::Normal(python_text.unwrap_or_default())
                };
                assert_eq!(rust_revision.text, expected_text, "text of {context}");

                let rust_sha1 = rust_revision.sha1.as_ref().map(|sha1| {
                    std::str::from_utf8(&sha1.0).expect("Rust parser stores SHA-1 as base36 ASCII")
                });
                assert_eq!(
                    rust_sha1,
                    python_revision
                        .getattr("sha1")?
                        .extract::<Option<String>>()?
                        .as_deref(),
                    "SHA-1 of {context}"
                );
                assert_eq!(
                    rust_revision.comment.as_deref(),
                    python_revision
                        .getattr("comment")?
                        .extract::<Option<String>>()?
                        .as_deref(),
                    "comment of {context}"
                );
                assert_eq!(
                    rust_revision.minor,
                    python_revision.getattr("minor")?.extract::<bool>()?,
                    "minor flag of {context}"
                );
            }
            assert_eq!(
                rust_page.revisions.len(),
                python_revision_count,
                "revision count of page {page_index} ({python_title:?})"
            );
        }

        assert!(
            rust_parser
                .parse_page()
                .expect("Rust parser failed after Python reached EOF")
                .is_none(),
            "Rust parser returned more pages than Python"
        );
        Ok(())
    })
}

#[cfg(feature = "python-diff")]
#[test]
fn python_parser_matches_rust_on_entity_references() {
    assert_parsers_match(DUMP_WITH_ENTITIES.as_bytes()).unwrap();
}

/// Compare every represented page and revision field against `mwxml`, the
/// official Python parser used by the reference WikiWho implementation.
#[cfg(feature = "python-diff")]
#[test]
fn python_parser_matches_rust_on_reference_dump() {
    let path =
        std::env::var("WIKIWHO_TEST_DUMP").unwrap_or_else(|_| DEFAULT_COMPARISON_DUMP.to_owned());
    let file = File::open(&path)
        .unwrap_or_else(|err| panic!("failed to open comparison dump `{path}`: {err}"));
    let xml = zstd::stream::decode_all(BufReader::new(file))
        .unwrap_or_else(|err| panic!("failed to decompress comparison dump `{path}`: {err}"));

    assert_parsers_match(&xml).unwrap();
}

/// Parse every page in a real Wikimedia history dump with strict validation.
///
/// CI points `WIKIWHO_TEST_DUMP` at the small representative dump on pull
/// requests and at the full bundled Wiktionary dump on pushes to `main`.
#[cfg(feature = "strict")]
#[test]
fn reference_dump_parses_completely_in_strict_mode() {
    let path =
        std::env::var("WIKIWHO_TEST_DUMP").unwrap_or_else(|_| DEFAULT_REFERENCE_DUMP.to_owned());
    let file = File::open(&path)
        .unwrap_or_else(|err| panic!("failed to open reference dump `{path}`: {err}"));
    let decoder = zstd::stream::Decoder::new(file)
        .unwrap_or_else(|err| panic!("failed to decompress reference dump `{path}`: {err}"));
    let mut parser = DumpParser::new(BufReader::new(decoder))
        .unwrap_or_else(|err| panic!("failed to parse site info from `{path}`: {err:?}"));

    let mut page_count = 0_u64;
    let mut revision_count = 0_u64;
    while let Some(page) = parser.parse_page().unwrap_or_else(|err| {
        panic!(
            "strict parsing failed after {page_count} pages at byte {} in `{path}`: {err:?}",
            parser.bytes_consumed()
        )
    }) {
        page_count += 1;
        revision_count += page.revisions.len() as u64;
    }

    assert!(page_count > 0, "reference dump `{path}` contained no pages");
    assert!(
        revision_count > 0,
        "reference dump `{path}` contained no revisions"
    );
    eprintln!("strictly parsed {page_count} pages and {revision_count} revisions from `{path}`");
}
