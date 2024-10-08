use std::{
    any::type_name_of_val,
    borrow::Cow,
    collections::HashMap,
    convert::Infallible,
    fmt::Debug,
    io::{BufRead, Read},
    sync::Arc,
};

use compact_str::CompactString;
use quick_xml::events::{BytesEnd, BytesStart};
use rand::Rng;
use tracing::instrument;

// we normally don't retrieve the value of the tags, so this is the most efficient backend
type TagStringInterner = string_interner::StringInterner<string_interner::backend::BucketBackend>;

// list of all tags that are revelevant for our use case
// i.e. the tags of which we need a value and their parent tags
#[derive(PartialEq, Eq)]
enum Tag {
    MediaWiki,  // <mediawiki version="0.11" ...other attributes>...</mediawiki> is the root tag
    SiteInfo, // <siteinfo><dbname>...</dbname><namespaces>...</namespaces> ...other tags</siteinfo>
    DbName,   // <dbname>dewiktionary</dbname>
    Namespaces, // <namespaces><namespace key="0" /> ...more namespace tags</namespaces>
    Namespace(String), // <namespace key="1">Diskussion</namespace>
    Page,     // <page>...tags are (title, ns, id, revision)</page>
    Title,    // <title>blah</title>
    Ns,       // <ns>0</ns>
    Id,       // <id>500</id>
    Revision, // <revision>...tags are (id, timestamp, contributor, text, sha1, comment, )</revision>
    Timestamp, // <timestamp>2003-12-05T06:41:50Z</timestamp>
    Contributor, // <contributor><username>blah</username><id>500</id></contributor>
    Username, // <username>blah</username>
    // Text's sha1 attribute seems to be preferred over the sha1 tag (https://github.com/mediawiki-utilities/python-mwxml/blob/2b477be6aa9794064d03b5be38c7759d1570488b/mwxml/iteration/revision.py#L83-L96)
    Text(bool, Option<String>), // <text bytes="20" sha1="3h3w...">blah</text> or <text bytes="20" sha1="3h3w..." deleted="deleted" />
    // Sha1 hash is base36 encoded (0-padded to 31 characters)
    Sha1,                                    // <sha1>3h3w...</sha1>
    Comment,                                 // <comment>blah</comment>
    Minor,                                   // <minor />
    Unknown(string_interner::DefaultSymbol), // any other tag
}

impl Debug for Tag {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Tag::MediaWiki => write!(f, "<mediawiki>"),
            Tag::SiteInfo => write!(f, "<siteinfo>"),
            Tag::DbName => write!(f, "<dbname>"),
            Tag::Namespaces => write!(f, "<namespaces>"),
            Tag::Namespace(key) => write!(f, "<namespace key={}>", key),
            Tag::Page => write!(f, "<page>"),
            Tag::Title => write!(f, "<title>"),
            Tag::Ns => write!(f, "<ns>"),
            Tag::Id => write!(f, "<id>"),
            Tag::Revision => write!(f, "<revision>"),
            Tag::Timestamp => write!(f, "<timestamp>"),
            Tag::Contributor => write!(f, "<contributor>"),
            Tag::Username => write!(f, "<username>"),
            Tag::Text(deleted, sha1) => {
                write!(f, "<text")?;
                if let Some(sha1) = sha1 {
                    write!(f, " sha1={:?}", sha1)?;
                }
                if *deleted {
                    write!(f, " deleted")?;
                }
                write!(f, ">")
            }
            Tag::Sha1 => write!(f, "<sha1>"),
            Tag::Comment => write!(f, "<comment>"),
            Tag::Minor => write!(f, "<minor>"),
            // TODO: find a way to retrieve the string for the interned symbol
            Tag::Unknown(tag) => write!(f, "<unknown tag - interned symbol: {:?}>", tag),
        }
    }
}

#[derive(Debug, thiserror::Error)]
enum TagReadingError<T> {
    /// Error returned to indicate that the input is not valid UTF-8.
    ///
    /// Allows continuing to parse the XML file, but this may lead to incorrect results if there is more than one distinct non-UTF-8 tag.
    #[error("non-UTF-8 tag detected")]
    NonUtf8Tag(T),
    #[error("XML error")]
    XmlError(#[from] quick_xml::Error),
    #[error("missing expected attribute `{0}` for tag `{1}`")]
    MissingAttribute(&'static str, &'static str),
}

#[derive(Debug, thiserror::Error)]
struct NonUtf8Tag<T>(T);

impl Tag {
    fn from_start_bytes(
        e: &BytesStart,
        tag_interner: &mut TagStringInterner,
    ) -> Result<Self, TagReadingError<Tag>> {
        match e.name().as_ref() {
            b"mediawiki" => Ok(Tag::MediaWiki),
            b"siteinfo" => Ok(Tag::SiteInfo),
            b"dbname" => Ok(Tag::DbName),
            b"namespaces" => Ok(Tag::Namespaces),
            b"namespace" => {
                for attr in e.attributes() {
                    let attr = attr.map_err(quick_xml::Error::from)?;

                    if attr.key.as_ref() == b"key" {
                        let key = attr.unescape_value()?;
                        return Ok(Tag::Namespace(key.into_owned()));
                    }
                }

                Err(TagReadingError::MissingAttribute("key", "namespace"))
            }
            b"page" => Ok(Tag::Page),
            b"title" => Ok(Tag::Title),
            b"ns" => Ok(Tag::Ns),
            b"id" => Ok(Tag::Id),
            b"revision" => Ok(Tag::Revision),
            b"timestamp" => Ok(Tag::Timestamp),
            b"contributor" => Ok(Tag::Contributor),
            b"username" => Ok(Tag::Username),
            b"text" => {
                let mut sha1 = None;
                let mut deleted = false;

                for attr in e.attributes() {
                    let attr = attr.map_err(quick_xml::Error::from)?;
                    match attr.key.as_ref() {
                        b"bytes" => {
                            let _bytes = attr.unescape_value()?;
                        }
                        b"sha1" => {
                            sha1 = Some(attr.unescape_value()?);
                        }
                        b"deleted" => {
                            deleted = true;
                        }
                        _ => {}
                    }
                }

                Ok(Tag::Text(deleted, sha1.map(Cow::into_owned)))
            }
            b"sha1" => Ok(Tag::Sha1),
            b"comment" => Ok(Tag::Comment),
            b"minor" => Ok(Tag::Minor),
            _ => {
                let name = e.name().into_inner();

                if let Ok(name) = std::str::from_utf8(name) {
                    Ok(Tag::Unknown(tag_interner.get_or_intern(name)))
                } else {
                    Err(TagReadingError::NonUtf8Tag(Tag::Unknown(
                        tag_interner.get_or_intern("non-utf8 tag"),
                    )))
                }
            }
        }
    }

    fn matches_end_bytes(
        &self,
        e: &quick_xml::events::BytesEnd,
        tag_interner: &mut TagStringInterner,
    ) -> Result<bool, NonUtf8Tag<bool>> {
        match (self, e.name().as_ref()) {
            (Tag::MediaWiki, b"mediawiki") => Ok(true),
            (Tag::SiteInfo, b"siteinfo") => Ok(true),
            (Tag::DbName, b"dbname") => Ok(true),
            (Tag::Namespaces, b"namespaces") => Ok(true),
            (Tag::Namespace(_), b"namespace") => Ok(true),
            (Tag::Page, b"page") => Ok(true),
            (Tag::Title, b"title") => Ok(true),
            (Tag::Ns, b"ns") => Ok(true),
            (Tag::Id, b"id") => Ok(true),
            (Tag::Revision, b"revision") => Ok(true),
            (Tag::Timestamp, b"timestamp") => Ok(true),
            (Tag::Contributor, b"contributor") => Ok(true),
            (Tag::Username, b"username") => Ok(true),
            (Tag::Text(_, _), b"text") => Ok(true),
            (Tag::Sha1, b"sha1") => Ok(true),
            (Tag::Comment, b"comment") => Ok(true),
            (Tag::Minor, b"minor") => Ok(true),
            (Tag::Unknown(expected_tag), tag_name) => {
                if let Ok(tag) = std::str::from_utf8(tag_name) {
                    let tag = tag_interner.get_or_intern(tag);
                    Ok(tag == *expected_tag)
                } else {
                    let tag = tag_interner.get_or_intern("non-utf8 tag");
                    Err(NonUtf8Tag(tag == *expected_tag))
                }
            }
            _ => Ok(false),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Contributor {
    pub username: CompactString,
    pub id: Option<i32>,
}

#[derive(Clone, PartialEq, Eq, Hash)]
pub enum Text {
    Normal(String),
    Deleted,
}

impl Text {
    pub fn len(&self) -> usize {
        match self {
            Text::Normal(text) => text.len(),
            Text::Deleted => 0,
        }
    }

    pub fn is_empty(&self) -> bool {
        match self {
            Text::Normal(text) => text.is_empty(),
            Text::Deleted => true,
        }
    }
}

impl Debug for Text {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Text::Normal(text) => write!(f, "{:?}", text),
            Text::Deleted => write!(f, "Deleted"),
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct Sha1Hash(pub(crate) [u8; 31]);

impl Debug for Sha1Hash {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if let Ok(as_str) = std::str::from_utf8(&self.0) {
            f.debug_tuple("Sha1Hash").field(&as_str).finish()
        } else {
            f.debug_tuple("Sha1Hash").field(&self.0).finish()
        }
    }
}

// apparently `restricted` is never set in mwxml (https://github.com/mediawiki-utilities/python-mwxml/blob/2b477be6aa9794064d03b5be38c7759d1570488b/mwxml/iteration/revision.py#L80)
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Revision {
    pub id: i32,
    pub timestamp: chrono::DateTime<chrono::Utc>,
    // aka. user
    pub contributor: Contributor,
    pub text: Text,
    pub sha1: Option<Sha1Hash>,
    pub comment: Option<CompactString>,
    pub minor: bool,
}

#[derive(Debug)]
struct RevisionBuilder {
    id: Option<i32>,
    timestamp: Option<chrono::DateTime<chrono::Utc>>,
    contributor_name: Option<CompactString>,
    contributor_id: Option<i32>,
    text: Option<Text>,
    sha1: Option<Sha1Hash>,
    comment: Option<CompactString>,
    minor: bool,
}

#[derive(Debug, thiserror::Error)]
#[error("missing mandatory field: {0}")]
struct BuildRevisionError(&'static str, Box<RevisionBuilder>);

impl RevisionBuilder {
    fn new() -> Self {
        Self {
            id: None,
            timestamp: None,
            contributor_name: None,
            contributor_id: None,
            text: None,
            sha1: None,
            comment: None,
            minor: false,
        }
    }

    fn try_build(self) -> Result<Revision, BuildRevisionError> {
        if self.id.is_none() {
            return Err(BuildRevisionError("id", self.into()));
        }
        if self.timestamp.is_none() {
            return Err(BuildRevisionError("timestamp", self.into()));
        }
        if self.contributor_name.is_none() {
            return Err(BuildRevisionError("contributor_name", self.into()));
        }
        if self.text.is_none() {
            return Err(BuildRevisionError("text", self.into()));
        }

        Ok(Revision {
            id: self.id.unwrap(),
            timestamp: self.timestamp.unwrap(),
            contributor: Contributor {
                username: self.contributor_name.unwrap(),
                id: self.contributor_id,
            },
            text: self.text.unwrap(),
            sha1: self.sha1,
            comment: self.comment,
            minor: self.minor,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Page {
    pub title: CompactString,
    pub namespace: i32,
    pub revisions: Vec<Revision>,
}

#[derive(Clone, PartialEq, Eq, Hash, Default)]
pub enum Namespace {
    #[default]
    Default,
    Named(CompactString),
}

impl Debug for Namespace {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Namespace::Default => write!(f, "Default"),
            Namespace::Named(name) => write!(f, "{:?}", name),
        }
    }
}

#[derive(Debug)]
pub struct SiteInfo {
    pub dbname: CompactString,
    pub namespaces: HashMap<i32, Namespace>,
}

pub struct DumpParser<R: BufRead> {
    tag_interner: TagStringInterner,
    xml_parser: quick_xml::Reader<R>,
    buf: Vec<u8>,
    current_path: Vec<Tag>,
    site_info: SiteInfo,
    non_utf8_reporter: NonUtf8Reporter,
}

impl<R: BufRead> Debug for DumpParser<R> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DumpParser")
            .field("tag_interner", &type_name_of_val(&self.tag_interner))
            .field("xml_parser", &type_name_of_val(&self.xml_parser))
            // print buffer length and capacity
            .field("buf.len", &self.buf.len())
            .field("buf.capacity", &self.buf.capacity())
            .field("current_path", &self.current_path)
            .field("site_info", &self.site_info)
            .finish()
    }
}

#[derive(Debug)]
struct NonUtf8Reporter {
    num_tags: usize,
}

impl NonUtf8Reporter {
    fn new() -> Self {
        Self { num_tags: 0 }
    }

    fn register(&mut self, name: &[u8]) {
        self.num_tags += 1;

        if self.num_tags == 1 {
            tracing::warn!(message = "Non-UTF-8 tag in XML detected. This is not expected. Parsing will continue, but the results may be incorrect. Further non-UTF-8 tags will not be reported.", name = String::from_utf8_lossy(name).as_ref());
        }
    }

    fn tag_from_start_bytes(
        &mut self,
        e: &BytesStart,
        tag_interner: &mut TagStringInterner,
    ) -> Result<Tag, TagReadingError<Infallible>> {
        match Tag::from_start_bytes(e, tag_interner) {
            Ok(tag) => Ok(tag),
            Err(TagReadingError::NonUtf8Tag(tag)) => {
                self.register(e.name().as_ref());

                if cfg!(feature = "strict") {
                    todo!("not sure how to abort parsing here");
                } else {
                    Ok(tag)
                }
            }
            Err(e) => match e {
                TagReadingError::NonUtf8Tag(_) => unreachable!(),
                TagReadingError::XmlError(e) => Err(TagReadingError::XmlError(e)),
                TagReadingError::MissingAttribute(att, tag) => {
                    Err(TagReadingError::MissingAttribute(att, tag))
                }
            },
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ParsingError {
    #[error("XML error")]
    XmlError(#[from] quick_xml::Error),
    #[error("unexpected end of file")]
    Eof,
}

// impl ParsingError {
//     fn is_recoverable(&self) -> bool {
//         match self {
//             ParsingError::XmlError(_) => todo!("decide if error is recoverable"),
//             ParsingError::Eof => false,
//         }
//     }
// }

impl<R: BufRead> DumpParser<R> {
    pub fn new(reader: R) -> Result<Self, ParsingError> {
        let xml_parser = quick_xml::Reader::from_reader(reader);
        //let config = xml_parser.config_mut();
        // expand_empty_elements not set, take care to handle empty elements!

        let mut new = Self {
            tag_interner: TagStringInterner::new(),
            xml_parser,
            // preallocate 1 MiB for the buffer
            buf: Vec::with_capacity(1024 * 1024),
            current_path: Vec::new(),
            site_info: SiteInfo {
                dbname: CompactString::default(),
                namespaces: HashMap::new(),
            },
            non_utf8_reporter: NonUtf8Reporter::new(),
        };

        new.parse_site_info()?;

        Ok(new)
    }

    pub fn site_info(&self) -> &SiteInfo {
        &self.site_info
    }

    #[instrument]
    fn parse_start_bytes(
        e: &BytesStart,
        expecting_namespace: bool,

        // unfortunately have to pass all these as arguments, because otherwise we get problems with the borrow checker
        non_utf8_reporter: &mut NonUtf8Reporter,
        tag_interner: &mut TagStringInterner,
        current_path: &[Tag],
    ) -> Result<Tag, quick_xml::Error> {
        match non_utf8_reporter.tag_from_start_bytes(e, tag_interner) {
            Ok(tag) => Ok(tag),
            Err(TagReadingError::MissingAttribute(attr, tag)) => {
                if tag == "namespace" {
                    if cfg!(feature = "strict") {
                        todo!();
                    }
                    // print warning and skip the tag
                    if expecting_namespace {
                        tracing::warn!(
                            message = "missing expected attribute, ignoring the namespace",
                            attribute = attr,
                            tag = tag
                        );
                    } else {
                        tracing::info!(
                            message = "found known tag in unexpected location",
                            tag = ?tag,
                            path = ?current_path
                        );
                    }
                    Ok(Tag::Namespace("ignored".to_string()))
                } else {
                    // unexpected
                    // TODO: adjust this if more tags get mandatory attributes
                    panic!(
                        "missing attribute for tag: {}, unexpected code flow, can't recover",
                        tag
                    );
                }
            }
            Err(TagReadingError::XmlError(e)) => {
                return Err(e);
            }
            _ => unreachable!(),
        }
    }

    // debugging aid for format changes
    fn check_known_tags_in_unexpected_location(&self, is_empty: bool) {
        let current_path = &self.current_path;

        if current_path.is_empty() {
            return;
        }

        let tag = current_path.last().unwrap();
        if !matches!(tag, Tag::Unknown(_)) {
            tracing::info!(
                message = "found known tag in unexpected location",
                tag = ?tag,
                path = ?current_path,
                is_empty
            );
        }
    }

    fn abort_parsing<T>(xml_parser: &mut quick_xml::Reader<R>) -> Result<T, ParsingError> {
        tracing::error!("Aborting parsing due to error");
        let mut useless_buf = [0];
        xml_parser
            .stream()
            .take(u64::MAX)
            .read(&mut useless_buf)
            .map_err(|e| quick_xml::Error::Io(Arc::new(e)))?;
        Err(ParsingError::Eof)
    }

    fn check_end_tag(
        e: &BytesEnd,
        current_path: &mut Vec<Tag>,
        tag_interner: &mut TagStringInterner,
        xml_parser: &mut quick_xml::Reader<R>,
    ) -> Result<Option<Tag>, ParsingError> {
        // error handling for mismatched tags
        let tag = if let Some(tag) = current_path.pop() {
            tag
        } else {
            let tag = String::from_utf8_lossy(e.name().into_inner());
            tracing::error!(message = "Unexpected end tag", tag = tag.as_ref(), current_path = ?current_path, position = xml_parser.buffer_position());

            if cfg!(feature = "strict") {
                return Self::abort_parsing(xml_parser);
            } else {
                tracing::warn!("Ignoring unexpected end tag. This may lead to incorrect results.");
                return Ok(None);
            }
        };

        // ignore non-utf8 error here because we already reported it when the tag was read
        //  (or it will not match the opening tag and we will report that anyway)
        let matches = tag
            .matches_end_bytes(e, tag_interner)
            .unwrap_or_else(|e| e.0);
        if !matches {
            tracing::error!(
                message = "Mismatched tags",
                expected = ?tag,
                actual = String::from_utf8_lossy(e.name().as_ref()).as_ref(),
                current_path = ?current_path,
                position = xml_parser.buffer_position()
            );

            if cfg!(feature = "strict") {
                return Self::abort_parsing(xml_parser);
            } else {
                tracing::warn!("Ignoring mismatched tag. This may lead to incorrect results.");

                // (1) either this closing tag does not have a corresponding opening tag,
                // (2) or it is not the expected closing tag (e.g. typo),
                // (3) or a previous opening tag is not closed
                // let's try to recover as best as possible

                // for (1) we would have to push the tag back onto the stack
                // for (2) we'd just continue
                // for (3) we'd need to find the corresponding opening tag and close it
                // we can't distinguish between these cases, so we'll just continue
            }
        }

        Ok(Some(tag))
    }

    #[instrument]
    fn parse_site_info(&mut self) -> Result<(), ParsingError> {
        let mut site_info = SiteInfo {
            dbname: CompactString::default(),
            namespaces: HashMap::new(),
        };

        loop {
            match self.xml_parser.read_event_into(&mut self.buf)? {
                quick_xml::events::Event::Start(ref e) => {
                    let tag = Self::parse_start_bytes(
                        e,
                        true,
                        &mut self.non_utf8_reporter,
                        &mut self.tag_interner,
                        &self.current_path,
                    )?;

                    self.current_path.push(tag);
                }
                quick_xml::events::Event::Empty(ref e) => {
                    let tag = Self::parse_start_bytes(
                        e,
                        true,
                        &mut self.non_utf8_reporter,
                        &mut self.tag_interner,
                        &self.current_path,
                    )?;

                    use Tag::*;

                    self.current_path.push(tag);
                    match self.current_path.as_slice() {
                        [MediaWiki, SiteInfo, Namespaces, Namespace(id)] => {
                            let key = if let Ok(id) = id.parse() {
                                id
                            } else {
                                tracing::warn!(
                                    message = "Ignoring namespace with invalid id",
                                    id,
                                    name = "ignored",
                                    position = self.xml_parser.buffer_position()
                                );
                                continue;
                            };
                            site_info.namespaces.insert(key, self::Namespace::Default);
                        }
                        _ => self.check_known_tags_in_unexpected_location(true),
                    }
                    self.current_path.pop();
                }
                quick_xml::events::Event::Text(e) => {
                    let text = e.unescape()?;

                    use Tag::*;

                    match self.current_path.as_slice() {
                        [MediaWiki, SiteInfo, DbName] => {
                            site_info.dbname = CompactString::from(text.as_ref());
                        }
                        [MediaWiki, SiteInfo, Namespaces, Namespace(id)] => {
                            let key = if let Ok(id) = id.parse() {
                                id
                            } else {
                                if id != "ignored" {
                                    tracing::warn!(
                                        message = "Ignoring namespace with invalid id",
                                        id,
                                        name = text.as_ref(),
                                        position = self.xml_parser.buffer_position()
                                    );
                                }
                                continue;
                            };
                            site_info.namespaces.insert(
                                key,
                                self::Namespace::Named(CompactString::from(text.as_ref())),
                            );
                        }
                        _ => self.check_known_tags_in_unexpected_location(false),
                    }
                }
                quick_xml::events::Event::End(ref e) => {
                    let tag = Self::check_end_tag(
                        e,
                        &mut self.current_path,
                        &mut self.tag_interner,
                        &mut self.xml_parser,
                    )?;

                    if tag == Some(Tag::SiteInfo) {
                        // found the closing tag for siteinfo, we're done
                        break;
                    }
                }
                quick_xml::events::Event::Eof => {
                    // we should never reach eof in a correct file because we break when we find the closing tag

                    tracing::error!(partial_site_info = ?site_info, current_path = ?self.current_path);
                    return Err(ParsingError::Eof);
                }
                _ => {}
            }
            self.buf.clear();
        }

        self.site_info = site_info;
        Ok(())
    }

    pub fn parse_page(&mut self) -> Result<Option<Page>, ParsingError> {
        let span = tracing::span!(tracing::Level::INFO, "parse_page", self=?self, title=tracing::field::Empty);

        let mut page = Page {
            title: CompactString::default(),
            namespace: 0,
            revisions: Vec::new(),
        };
        let mut started_page = false;

        let mut revision_builder = None;

        loop {
            match self.xml_parser.read_event_into(&mut self.buf)? {
                quick_xml::events::Event::Start(ref e) => {
                    let tag = Self::parse_start_bytes(
                        e,
                        false,
                        &mut self.non_utf8_reporter,
                        &mut self.tag_interner,
                        &self.current_path,
                    )?;

                    if tag == Tag::Page {
                        started_page = true;
                    }

                    if tag == Tag::Revision {
                        revision_builder = Some(RevisionBuilder::new());
                    }

                    self.current_path.push(tag);
                }
                quick_xml::events::Event::Empty(ref e) => {
                    let tag = Self::parse_start_bytes(
                        e,
                        false,
                        &mut self.non_utf8_reporter,
                        &mut self.tag_interner,
                        &self.current_path,
                    )?;

                    self.current_path.push(tag);

                    use Tag::*;

                    match self.current_path.as_slice() {
                        // Revision tags
                        [MediaWiki, Page, Revision, Text(_, _)] => {
                            // empty text tag
                            if let Some(revision_builder) = &mut revision_builder {
                                revision_builder.text = Some(self::Text::Normal(String::new()));
                            }
                        }
                        [MediaWiki, Page, Revision, Minor] => {
                            // minor tag is always empty
                            if let Some(revision_builder) = &mut revision_builder {
                                revision_builder.minor = true;
                            }
                        }
                        _ => self.check_known_tags_in_unexpected_location(true),
                    }
                    self.current_path.pop();
                }
                quick_xml::events::Event::Text(e) => {
                    let text = e.unescape()?;

                    use Tag::*;

                    match self.current_path.as_slice() {
                        // Page tags
                        [MediaWiki, Page, Title] => {
                            fn normalize_title(title: &str) -> Cow<'_, str> {
                                if title.contains("_") {
                                    title.replace("_", " ").into()
                                } else {
                                    title.into()
                                }
                            }

                            if let Some(title) = text.split_once(":") {
                                // split off the namespace
                                page.title = CompactString::from(normalize_title(title.1));
                            } else {
                                page.title = CompactString::from(normalize_title(&text));
                            }
                            span.record("title", page.title.as_str());
                        }
                        [MediaWiki, Page, Ns] => {
                            let ns = if let Ok(id) = text.parse() {
                                id
                            } else {
                                tracing::warn!(
                                    message = "Found invalid namespace id, defaulting to 0",
                                    ns = text.as_ref(),
                                    position = self.xml_parser.buffer_position()
                                );
                                0
                            };
                            page.namespace = ns;
                        }
                        // Revision tags
                        [MediaWiki, Page, Revision, Id] => {
                            if let Some(revision_builder) = &mut revision_builder {
                                revision_builder.id = if let Ok(id) = text.parse() {
                                    Some(id)
                                } else {
                                    tracing::info!(
                                        message =
                                            "Found invalid revision id, generating a random id",
                                        id = text.as_ref(),
                                        position = self.xml_parser.buffer_position()
                                    );
                                    // always use negative ids for invalid ids
                                    Some(rand::thread_rng().gen_range(i32::MIN..-100))
                                };
                            }
                        }
                        [MediaWiki, Page, Revision, Timestamp] => {
                            // Source: https://github.com/mediawiki-utilities/python-mwtypes/blob/523a93f98fe1372938fc15872b5abb1f267cc643/mwtypes/timestamp.py#L12
                            const TIMESTAMP_FORMAT_LONG: &str = "%Y-%m-%dT%H:%M:%SZ";
                            const TIMESTAMP_FORMAT_SHORT: &str = "%Y%m%d%H%M%S";

                            if let Some(revision_builder) = &mut revision_builder {
                                revision_builder.timestamp = if let Ok(timestamp) =
                                    chrono::NaiveDateTime::parse_from_str(
                                        text.as_ref(),
                                        TIMESTAMP_FORMAT_SHORT,
                                    )
                                    .or_else(|_| {
                                        chrono::NaiveDateTime::parse_from_str(
                                            text.as_ref(),
                                            TIMESTAMP_FORMAT_LONG,
                                        )
                                    })
                                    .map(|dt| {
                                        chrono::DateTime::from_naive_utc_and_offset(dt, chrono::Utc)
                                    }) {
                                    Some(timestamp)
                                } else {
                                    tracing::warn!(
                                        message = "Found invalid revision timestamp",
                                        timestamp = text.as_ref(),
                                        position = self.xml_parser.buffer_position()
                                    );
                                    None
                                };
                            }
                        }
                        [MediaWiki, Page, Revision, Contributor, Username] => {
                            if let Some(revision_builder) = &mut revision_builder {
                                revision_builder.contributor_name =
                                    Some(CompactString::from(text.as_ref()));
                            }
                        }
                        [MediaWiki, Page, Revision, Contributor, Id] => {
                            if let Some(revision_builder) = &mut revision_builder {
                                revision_builder.contributor_id = if let Ok(id) = text.parse() {
                                    Some(id)
                                } else {
                                    tracing::warn!(
                                        message = "Found invalid contributor id",
                                        id = text.as_ref(),
                                        position = self.xml_parser.buffer_position()
                                    );
                                    None
                                };
                            }
                        }
                        [MediaWiki, Page, Revision, Text(deleted, _)] => {
                            if let Some(revision_builder) = &mut revision_builder {
                                revision_builder.text = Some(if *deleted {
                                    self::Text::Deleted
                                } else {
                                    self::Text::Normal(text.into_owned())
                                });
                            }
                        }
                        [MediaWiki, Page, Revision, Sha1] => {
                            if let Some(revision_builder) = &mut revision_builder {
                                let mut sha1 = [0; 31];
                                let bytes = text.as_bytes();
                                if bytes.len() == 31 {
                                    sha1.copy_from_slice(bytes);
                                    revision_builder.sha1 = Some(Sha1Hash(sha1));
                                } else {
                                    tracing::warn!(
                                        message = "Found invalid sha1 hash",
                                        sha1 = text.as_ref(),
                                        position = self.xml_parser.buffer_position()
                                    );
                                }
                            }
                        }
                        [MediaWiki, Page, Revision, Comment] => {
                            if let Some(revision_builder) = &mut revision_builder {
                                revision_builder.comment = Some(CompactString::from(text.as_ref()));
                            }
                        }
                        [MediaWiki, Page, Revision, Minor] => {
                            // minor tag should be empty, but just in case it's not handle it here as well
                            if let Some(revision_builder) = &mut revision_builder {
                                revision_builder.minor = true;
                            }
                        }
                        _ => self.check_known_tags_in_unexpected_location(false),
                    }
                }
                quick_xml::events::Event::End(ref e) => {
                    let tag = Self::check_end_tag(
                        e,
                        &mut self.current_path,
                        &mut self.tag_interner,
                        &mut self.xml_parser,
                    )?;

                    if tag == Some(Tag::Revision) {
                        if let Some(revision_builder) = revision_builder.take() {
                            let revision = match revision_builder.try_build() {
                                Ok(revision) => revision,
                                Err(BuildRevisionError(field, revision_builder)) => {
                                    tracing::error!(
                                        message = "Missing mandatory field in revision",
                                        field,
                                        partial_revision = ?revision_builder,
                                        revision_end_position = self.xml_parser.buffer_position()
                                    );
                                    if cfg!(feature = "strict") {
                                        return Self::abort_parsing(&mut self.xml_parser);
                                    } else {
                                        tracing::warn!(
                                            "Ignoring revision with missing mandatory field"
                                        );
                                        continue;
                                    }
                                }
                            };
                            page.revisions.push(revision);
                        }
                    }

                    if tag == Some(Tag::Page) {
                        break;
                    }
                }
                quick_xml::events::Event::Eof => {
                    if started_page {
                        tracing::error!(partial_page = ?page, current_path = ?self.current_path);
                        return Err(ParsingError::Eof);
                    } else {
                        return Ok(None);
                    }
                }
                _ => {}
            }
            self.buf.clear();
        }

        Ok(Some(page))
    }
}
