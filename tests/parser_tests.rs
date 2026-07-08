// SPDX-License-Identifier: MPL-2.0
//! Parser integration tests.
//!
//! Broader parsing coverage (e.g. the full bundled Wiktionary dump in `strict`
//! mode) is tracked in <https://github.com/Schuwi/wikiwho_rs/issues/6>.

use std::io::Cursor;
use wikiwho::dump_parser::DumpParser;

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
			<text xml:space="preserve">AAA words here. &lt;ref&gt;x&lt;/ref&gt; BBB words here.</text>
		</revision>
	</page>
</mediawiki>"#;

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
}
