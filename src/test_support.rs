// SPDX-License-Identifier: MPL-2.0
//! All tests need to be run in a Python venv that has installed the `requirements.txt`!

use chrono::DateTime;
use pyo3::FromPyObject;
use quick_xml::events::{BytesEnd, BytesStart, BytesText, Event};
use std::{collections::HashMap, io::Cursor};

use crate::dump_parser::{Contributor, Page, Revision, Text};

pub mod prelude {
    pub(crate) use super::proptest as proptest_support;
    pub(crate) use super::{dummy_revision, page_to_xml, with_gil};
    pub(crate) use proptest::prelude::*;
    pub(crate) use pyo3::prelude::*;
}

pub use delta_debugging::delta_debug_texts;

macro_rules! with_gil {
    ($py: ident, $body: expr) => {{
        let result = Python::with_gil(|$py| {
            let _: () = $body;
            Ok(())
        });
        // workaround for prop_assert! not working correctly in Python::with_gil
        if result.is_err() {
            return result;
        }
    }};
}
pub(crate) use with_gil;

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

#[derive(FromPyObject)]
pub struct PyWikiwho {
    pub spam_ids: Vec<i32>,
    pub revisions: HashMap<i32, PyRevision>,
    pub ordered_revisions: Vec<i32>,
    pub revision_curr: PyRevision,
}

#[derive(FromPyObject)]
pub struct PyRevision {
    pub id: i32,
    pub paragraphs: HashMap<String, Vec<PyParagraph>>,
    pub ordered_paragraphs: Vec<String>,
    pub original_adds: usize,
}

#[derive(FromPyObject)]
pub struct PyParagraph {
    pub value: String,
    pub sentences: HashMap<String, Vec<PySentence>>,
    pub ordered_sentences: Vec<String>,
}

#[derive(FromPyObject)]
pub struct PySentence {
    pub value: String,
    pub words: Vec<PyWord>,
}

#[derive(FromPyObject)]
pub struct PyWord {
    pub token_id: i32,
    pub value: String,
    pub origin_rev_id: i32,
    pub last_rev_id: i32,
    pub outbound: Vec<i32>,
    pub inbound: Vec<i32>,
}

pub fn page_to_xml(page: &Page) -> String {
    //     const HEADER: &str = r#"<mediawiki xmlns="http://www.mediawiki.org/xml/export-0.11/" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance" xsi:schemaLocation="http://www.mediawiki.org/xml/export-0.11/ http://www.mediawiki.org/xml/export-0.11.xsd" version="0.11" xml:lang="de">
    //   <siteinfo>
    //     <sitename>Wiktionary</sitename>
    //     <dbname>dewiktionary</dbname>
    //     <base>https://de.wiktionary.org/wiki/Wiktionary:Hauptseite</base>
    //     <generator>MediaWiki 1.43.0-wmf.20</generator>
    //     <case>case-sensitive</case>
    //     <namespaces>
    //       <namespace key="-2" case="case-sensitive">Medium</namespace>
    //       <namespace key="-1" case="first-letter">Spezial</namespace>
    //       <namespace key="0" case="case-sensitive" />
    //       <namespace key="1" case="case-sensitive">Diskussion</namespace>
    //       <namespace key="2" case="first-letter">Benutzer</namespace>
    //       <namespace key="3" case="first-letter">Benutzer Diskussion</namespace>
    //       <namespace key="4" case="case-sensitive">Wiktionary</namespace>
    //       <namespace key="5" case="case-sensitive">Wiktionary Diskussion</namespace>
    //       <namespace key="6" case="case-sensitive">Datei</namespace>
    //       <namespace key="7" case="case-sensitive">Datei Diskussion</namespace>
    //       <namespace key="8" case="first-letter">MediaWiki</namespace>
    //       <namespace key="9" case="first-letter">MediaWiki Diskussion</namespace>
    //       <namespace key="10" case="case-sensitive">Vorlage</namespace>
    //       <namespace key="11" case="case-sensitive">Vorlage Diskussion</namespace>
    //       <namespace key="12" case="case-sensitive">Hilfe</namespace>
    //       <namespace key="13" case="case-sensitive">Hilfe Diskussion</namespace>
    //       <namespace key="14" case="case-sensitive">Kategorie</namespace>
    //       <namespace key="15" case="case-sensitive">Kategorie Diskussion</namespace>
    //       <namespace key="102" case="case-sensitive">Verzeichnis</namespace>
    //       <namespace key="103" case="case-sensitive">Verzeichnis Diskussion</namespace>
    //       <namespace key="104" case="case-sensitive">Thesaurus</namespace>
    //       <namespace key="105" case="case-sensitive">Thesaurus Diskussion</namespace>
    //       <namespace key="106" case="case-sensitive">Reim</namespace>
    //       <namespace key="107" case="case-sensitive">Reim Diskussion</namespace>
    //       <namespace key="108" case="case-sensitive">Flexion</namespace>
    //       <namespace key="109" case="case-sensitive">Flexion Diskussion</namespace>
    //       <namespace key="110" case="case-sensitive">Rekonstruktion</namespace>
    //       <namespace key="111" case="case-sensitive">Rekonstruktion Diskussion</namespace>
    //       <namespace key="710" case="case-sensitive">TimedText</namespace>
    //       <namespace key="711" case="case-sensitive">TimedText talk</namespace>
    //       <namespace key="828" case="case-sensitive">Modul</namespace>
    //       <namespace key="829" case="case-sensitive">Modul Diskussion</namespace>
    //     </namespaces>
    //   </siteinfo>
    //   "#;

    // const FOOTER: &str = r#"</mediawiki>"#;

    // Source: https://github.com/mediawiki-utilities/python-mwtypes/blob/523a93f98fe1372938fc15872b5abb1f267cc643/mwtypes/timestamp.py#L12
    const TIMESTAMP_FORMAT_LONG: &str = "%Y-%m-%dT%H:%M:%SZ";

    // let mut xml = HEADER.to_string();
    let mut xml = Vec::new();
    let mut writer = quick_xml::Writer::new(Cursor::new(&mut xml));
    writer
        .write_event(Event::Start(BytesStart::new("page")))
        .unwrap();

    writer
        .write_event(Event::Start(BytesStart::new("title")))
        .unwrap();
    // if let Some(site_info) = site_info {
    //     let namespace = site_info.namespaces.get(&page.namespace);
    //     if let Some(Namespace::Named(namespace)) = namespace {
    //         writer
    //             .write_event(Event::Text(BytesText::new(&format!(
    //                 "{}:{}",
    //                 namespace, page.title
    //             ))))
    //             .unwrap();
    //     } else {
    //         writer
    //             .write_event(Event::Text(BytesText::new(&page.title)))
    //             .unwrap();
    //     }
    // } else {
    //     writer
    //         .write_event(Event::Text(BytesText::new(&page.title)))
    //         .unwrap();
    // }
    writer
        .write_event(Event::Text(BytesText::new(&page.title)))
        .unwrap();
    writer
        .write_event(Event::End(BytesEnd::new("title")))
        .unwrap();

    writer
        .write_event(Event::Start(BytesStart::new("ns")))
        .unwrap();
    // writer
    //     .write_event(Event::Text(BytesText::new(&page.namespace.to_string())))
    //     .unwrap();
    // namespaces are not supported by python if using `Dump.from_page_xml` (i.e. the `siteinfo` is not present)
    writer
        .write_event(Event::Text(BytesText::new("0")))
        .unwrap();
    writer.write_event(Event::End(BytesEnd::new("ns"))).unwrap();

    writer
        .write_event(Event::Start(BytesStart::new("id")))
        .unwrap();
    writer
        .write_event(Event::Text(BytesText::new(&"20".to_string())))
        .unwrap(); /* ignored in algorithm */
    writer.write_event(Event::End(BytesEnd::new("id"))).unwrap();

    for revision in &page.revisions {
        writer
            .write_event(Event::Start(BytesStart::new("revision")))
            .unwrap();

        writer
            .write_event(Event::Start(BytesStart::new("id")))
            .unwrap();
        writer
            .write_event(Event::Text(BytesText::new(&revision.id.to_string())))
            .unwrap();
        writer.write_event(Event::End(BytesEnd::new("id"))).unwrap();

        writer
            .write_event(Event::Start(BytesStart::new("origin")))
            .unwrap();
        writer
            .write_event(Event::Text(BytesText::new(&revision.id.to_string())))
            .unwrap();
        writer
            .write_event(Event::End(BytesEnd::new("origin")))
            .unwrap();

        writer
            .write_event(Event::Start(BytesStart::new("model")))
            .unwrap();
        writer
            .write_event(Event::Text(BytesText::new("wikitext")))
            .unwrap();
        writer
            .write_event(Event::End(BytesEnd::new("model")))
            .unwrap();

        writer
            .write_event(Event::Start(BytesStart::new("format")))
            .unwrap();
        writer
            .write_event(Event::Text(BytesText::new("text/x-wiki")))
            .unwrap();
        writer
            .write_event(Event::End(BytesEnd::new("format")))
            .unwrap();

        writer
            .write_event(Event::Start(BytesStart::new("timestamp")))
            .unwrap();
        writer
            .write_event(Event::Text(BytesText::new(
                &revision.timestamp.format(TIMESTAMP_FORMAT_LONG).to_string(),
            )))
            .unwrap();
        writer
            .write_event(Event::End(BytesEnd::new("timestamp")))
            .unwrap();

        writer
            .write_event(Event::Start(BytesStart::new("contributor")))
            .unwrap();
        writer
            .write_event(Event::Start(BytesStart::new("username")))
            .unwrap();
        writer
            .write_event(Event::Text(BytesText::new(&revision.contributor.username)))
            .unwrap();
        writer
            .write_event(Event::End(BytesEnd::new("username")))
            .unwrap();
        if let Some(id) = revision.contributor.id {
            writer
                .write_event(Event::Start(BytesStart::new("id")))
                .unwrap();
            writer
                .write_event(Event::Text(BytesText::new(&id.to_string())))
                .unwrap();
            writer.write_event(Event::End(BytesEnd::new("id"))).unwrap();
        }
        writer
            .write_event(Event::End(BytesEnd::new("contributor")))
            .unwrap();

        match (&revision.text, &revision.sha1) {
            (Text::Normal(text), Some(sha1)) => {
                let bytes_str = text.len().to_string();
                let attributes = vec![
                    ("xml:space", "preserve"),
                    ("bytes", &bytes_str),
                    ("sha1", std::str::from_utf8(&sha1.0).unwrap()),
                ];

                writer
                    .write_event(Event::Start(
                        BytesStart::new("text").with_attributes(attributes.into_iter()),
                    ))
                    .unwrap();
                writer
                    .write_event(Event::Text(BytesText::new(text)))
                    .unwrap();
                writer
                    .write_event(Event::End(BytesEnd::new("text")))
                    .unwrap();
            }
            (Text::Normal(text), None) => {
                let bytes_str = text.len().to_string();
                let attributes = vec![("xml:space", "preserve"), ("bytes", &bytes_str)];

                writer
                    .write_event(Event::Start(
                        BytesStart::new("text").with_attributes(attributes.into_iter()),
                    ))
                    .unwrap();
                writer
                    .write_event(Event::Text(BytesText::new(text)))
                    .unwrap();
                writer
                    .write_event(Event::End(BytesEnd::new("text")))
                    .unwrap();
            }
            (Text::Deleted, Some(sha1)) => {
                let attributes = vec![
                    ("xml:space", "preserve"),
                    ("bytes", "0"),
                    ("sha1", std::str::from_utf8(&sha1.0).unwrap()),
                    ("deleted", "deleted"),
                ];

                writer
                    .write_event(Event::Start(
                        BytesStart::new("text").with_attributes(attributes.into_iter()),
                    ))
                    .unwrap();
                writer
                    .write_event(Event::End(BytesEnd::new("text")))
                    .unwrap();
            }
            (Text::Deleted, None) => {
                let attributes = vec![
                    ("xml:space", "preserve"),
                    ("bytes", "0"),
                    ("deleted", "deleted"),
                ];

                writer
                    .write_event(Event::Empty(
                        BytesStart::new("text").with_attributes(attributes.into_iter()),
                    ))
                    .unwrap();
            }
        }
        if let Some(sha1) = &revision.sha1 {
            writer
                .write_event(Event::Start(BytesStart::new("sha1")))
                .unwrap();
            writer
                .write_event(Event::Text(BytesText::new(
                    std::str::from_utf8(&sha1.0).unwrap(),
                )))
                .unwrap();
            writer
                .write_event(Event::End(BytesEnd::new("sha1")))
                .unwrap();
        }
        if let Some(comment) = &revision.comment {
            writer
                .write_event(Event::Start(BytesStart::new("comment")))
                .unwrap();
            writer
                .write_event(Event::Text(BytesText::new(comment)))
                .unwrap();
            writer
                .write_event(Event::End(BytesEnd::new("comment")))
                .unwrap();
        }
        if revision.minor {
            writer
                .write_event(Event::Empty(BytesStart::new("minor")))
                .unwrap();
        }
        writer
            .write_event(Event::End(BytesEnd::new("revision")))
            .unwrap();
    }
    writer
        .write_event(Event::End(BytesEnd::new("page")))
        .unwrap();
    writer.write_event(Event::Eof).unwrap();

    // xml.push_str(FOOTER);

    // println!("{}", xml);

    String::from_utf8(xml).unwrap()
}

pub mod proptest {
    use compact_str::CompactString;
    use proptest::prelude::*;
    use proptest::strategy::Strategy;

    use crate::dump_parser::{Contributor, Page, Revision, Sha1Hash, Text};

    pub fn maybe_comment() -> impl Strategy<Value = Option<CompactString>> {
        prop_oneof![
            7 => Just(None),
            1 => any::<String>().prop_map(CompactString::from).prop_map(Some)
        ]
    }

    pub fn correct_text(text_strategy: BoxedStrategy<String>) -> impl Strategy<Value = Text> {
        prop_oneof![
            1 => Just(Text::Deleted),
            3 => text_strategy.prop_map(|s| Text::Normal(s))
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

pub mod delta_debugging {
    use std::collections::HashSet;

    use crate::{
        dump_parser::{Page, Text},
        test_support::page_to_xml,
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
                println!("Simplified individually: {}", page_to_xml(&candidate));
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
                println!("Simplified jointly: {}", page_to_xml(&candidate));
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
