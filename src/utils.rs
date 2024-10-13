use aho_corasick::{AhoCorasick, AhoCorasickBuilder, PatternID};
use imara_diff::{
    intern::{Interner, Token},
    Algorithm,
};
use memchr::memmem;

#[allow(dead_code)] // it IS used in `split_into_tokens_corasick`
const fn const_str_equals(a: &str, b: &str) -> bool {
    let mut i = 0;
    while i < a.len() && i < b.len() {
        if a.as_bytes()[i] != b.as_bytes()[i] {
            return false;
        }
        i += 1;
    }
    i == a.len() && i == b.len()
}

/// Replace all occurrences of `from` with `to` in `input`.
///
/// This function is optimized for the case where no replacements are made.
///
/// # Arguments
///
/// * `input` - The input string to search for replacements.
/// * `from` - The `Finder` to search for. Must be created from valid UTF-8.
/// * `to` - The string to replace `from` with.
/// * `scratch_buffer` - A buffer to store the result in. Is expected to be empty.
///
/// # Returns
///
/// A tuple containing the modified `input` and the `clear`ed `scratch_buffer`.
///
/// # Panics
///
/// Might panic if `from` is not valid UTF-8.
fn str_replace_opt(
    input: String,
    from: &memmem::Finder,
    to: &str,
    scratch_buffer: String,
) -> (String, String) {
    let mut _ignored = false;
    str_replace_opt_ext(input, from, to, scratch_buffer, &mut _ignored)
}

fn str_replace_opt_ext(
    mut input: String,
    from: &memmem::Finder,
    to: &str,
    scratch_buffer: String,
    did_replace: &mut bool,
) -> (String, String) {
    let mut result = scratch_buffer;
    let mut last_end = 0;
    for m in from.find_iter(input.as_bytes()) {
        let start = m;
        let end = start + from.needle().len();

        // string indexing could panic if the Finder is not valid UTF-8
        result.push_str(&input[last_end..start]);
        result.push_str(to);

        last_end = end;
    }

    if last_end == 0 {
        // no replacements were made
        *did_replace = false;
        // no need to clear the scratch buffer, since it's already empty
        (input, result)
    } else {
        *did_replace = true;

        // copy the remaining text
        result.push_str(&input[last_end..]);

        input.clear();
        (result, input)
    }
}

macro_rules! finder {
    ($needle:expr) => {{
        static FINDER: LazyLock<memmem::Finder> =
            LazyLock::new(|| memmem::Finder::new($needle.as_bytes()));
        &FINDER
    }};
}

/// Find all `regex` matches in `input` and replace them with the result of `replacement`.
///
/// This function is optimized for the case where no replacements are made and intended for `replacement`s
/// that have capture groups. For `replacement`s that don't have capture groups, further optimization is possible.
///
/// # Arguments
///
/// * `input` - The input string to search for replacements.
/// * `regex` - The regex to search for.
/// * `replacement` - The replacer to use for replacements.
/// * `scratch_buffer` - A buffer to store the result in. Is expected to be empty.
///
/// # Returns
///
/// A tuple containing the modified `input` and the `clear`ed `scratch_buffer`.
fn regex_replace_opt<R: regex::Replacer>(
    mut input: String,
    regex: &Regex,
    mut replacement: R,
    scratch_buffer: String,
) -> (String, String) {
    let mut capt_iter = regex.captures_iter(&input).peekable();

    if capt_iter.peek().is_none() {
        // no matches found, return early

        // no need to clear the scratch buffer, since it's already empty
        (input, scratch_buffer)
    } else {
        let mut result = scratch_buffer;
        let mut last_end = 0;
        for cap in capt_iter {
            let m = cap.get(0).unwrap();
            let start = m.start();
            let end = m.end();

            result.push_str(&input[last_end..start]);
            replacement.replace_append(&cap, &mut result);

            last_end = end;
        }

        // copy the remaining text
        result.push_str(&input[last_end..]);

        input.clear();
        (result, input)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum RevisionHash {
    Sha1(Sha1Hash),
    Blake3(blake3::Hash),
}

/// Split the input text into paragraphs.
///
/// # Arguments
///
/// * `text` - The input text to split.
/// * `scratch_buffers` - A tuple containing two scratch buffers to use for temporary storage.
///                       They must be empty and will again be empty after the function returns.
///                       They should be reused across multiple calls to this function.
pub fn split_into_paragraphs(
    text: &str,
    scratch_buffers: (&mut String, &mut String),
) -> Vec<String> {
    if cfg!(feature = "optimized-str") {
        split_into_paragraphs_optimized(text, scratch_buffers)
    } else {
        split_into_paragraphs_naive(text)
    }
}

#[doc(hidden)] /* only public for benchmarking */
pub fn split_into_paragraphs_naive(text: &str) -> Vec<String> {
    let text = text.replace("\r\n", "\n").replace("\r", "\n");

    let text = text
        .replace("<table>", "\n\n<table>")
        .replace("</table>", "</table>\n\n");

    let text = text
        .replace("<tr>", "\n\n<tr>")
        .replace("</tr>", "</tr>\n\n");

    let text = text.replace("{|", "\n\n{|").replace("|}", "|}\n\n");
    let text = text.replace("|-\n", "\n\n|-\n");

    text.split("\n\n").map(|s| s.to_string()).collect()
}

#[doc(hidden)] /* only public for benchmarking */
pub fn split_into_paragraphs_optimized(
    text: &str,
    scratch_buffers: (&mut String, &mut String),
) -> Vec<String> {
    scratch_buffers.0.push_str(text);

    let (text, scratch_buffer) = (
        std::mem::take(scratch_buffers.0),
        std::mem::take(scratch_buffers.1),
    );

    let (text, scratch_buffer) = str_replace_opt(text, finder!("\r\n"), "\n", scratch_buffer);
    let (text, scratch_buffer) = str_replace_opt(text, finder!("\r"), "\n", scratch_buffer);

    let (text, scratch_buffer) =
        str_replace_opt(text, finder!("<table>"), "\n\n<table>", scratch_buffer);
    let (text, scratch_buffer) =
        str_replace_opt(text, finder!("</table>"), "</table>\n\n", scratch_buffer);

    let (text, scratch_buffer) = str_replace_opt(text, finder!("<tr>"), "\n\n<tr>", scratch_buffer);
    let (text, scratch_buffer) =
        str_replace_opt(text, finder!("</tr>"), "</tr>\n\n", scratch_buffer);

    let (text, scratch_buffer) = str_replace_opt(text, finder!("{|"), "\n\n{|", scratch_buffer);
    let (text, scratch_buffer) = str_replace_opt(text, finder!("|}"), "|}\n\n", scratch_buffer);

    let (text, scratch_buffer) = str_replace_opt(text, finder!("|-\n"), "\n\n|-\n", scratch_buffer);

    let result = text.split("\n\n").map(|s| s.to_string()).collect();

    let mut text = text;
    text.clear();
    // scratch_buffer is already empty

    *scratch_buffers.0 = text;
    *scratch_buffers.1 = scratch_buffer;

    result
}

use regex::Regex;

pub fn split_into_sentences(
    text: &str,
    scratch_buffers: (&mut String, &mut String),
) -> Vec<String> {
    if cfg!(feature = "optimized-str") {
        split_into_sentences_optimized(text, scratch_buffers)
    } else {
        split_into_sentences_naive(text)
    }
}

#[doc(hidden)] /* only public for benchmarking */
pub fn split_into_sentences_naive(text: &str) -> Vec<String> {
    static REGEX_DOT: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"([^\s\.=][^\s\.=][^\s\.=]\.) ").unwrap());
    static REGEX_URL: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"(http.*?://.*?[ \|<>\n\r])").unwrap());

    let text = text.replace("\n", "\n@@@@");
    let text = REGEX_DOT.replace_all(&text, "$1@@@@");
    let text = text.replace("; ", ";@@@@");
    let text = text.replace("? ", "?@@@@");
    let text = text.replace("! ", "!@@@@");
    let text = text.replace(": ", ":@@@@");
    let text = text.replace("\t", "\t@@@@");
    let text = text.replace("<!--", "@@@@<!--");
    let text = text.replace("-->", "-->@@@@");
    let text = text.replace("<ref", "@@@@<ref");
    let text = text.replace("/ref>", "/ref>@@@@");
    let text = REGEX_URL.replace_all(&text, "@@@@$1@@@@");

    let mut text = text.into_owned();
    while text.contains("@@@@@@@@") {
        text = text.replace("@@@@@@@@", "@@@@");
    }

    text.split("@@@@").map(|s| s.to_string()).collect()
}

pub fn split_into_sentences_optimized(
    text: &str,
    scratch_buffers: (&mut String, &mut String),
) -> Vec<String> {
    static REGEX_DOT: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"([^\s\.=][^\s\.=][^\s\.=]\.) ").unwrap());
    static REGEX_URL: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"(http.*?://.*?[ \|<>\n\r])").unwrap());

    scratch_buffers.0.push_str(text);

    let (text, scratch_buffer) = (
        std::mem::take(scratch_buffers.0),
        std::mem::take(scratch_buffers.1),
    );

    let (text, scratch_buffer) = str_replace_opt(text, finder!("\n"), "\n@@@@", scratch_buffer);

    let (text, scratch_buffer) = regex_replace_opt(text, &REGEX_DOT, "$1@@@@", scratch_buffer);

    let (text, scratch_buffer) = str_replace_opt(text, finder!("; "), ";@@@@", scratch_buffer);
    let (text, scratch_buffer) = str_replace_opt(text, finder!("? "), "?@@@@", scratch_buffer);
    let (text, scratch_buffer) = str_replace_opt(text, finder!("! "), "!@@@@", scratch_buffer);
    let (text, scratch_buffer) = str_replace_opt(text, finder!(": "), ":@@@@", scratch_buffer);
    let (text, scratch_buffer) = str_replace_opt(text, finder!("\t"), "\t@@@@", scratch_buffer);

    let (text, scratch_buffer) = str_replace_opt(text, finder!("<!--"), "@@@@<!--", scratch_buffer);
    let (text, scratch_buffer) = str_replace_opt(text, finder!("-->"), "-->@@@@", scratch_buffer);
    let (text, scratch_buffer) = str_replace_opt(text, finder!("<ref"), "@@@@<ref", scratch_buffer);
    let (text, scratch_buffer) =
        str_replace_opt(text, finder!("/ref>"), "/ref>@@@@", scratch_buffer);

    let (text, scratch_buffer) = regex_replace_opt(text, &REGEX_URL, "@@@@$1@@@@", scratch_buffer);

    let (mut text, mut scratch_buffer) = (text, scratch_buffer);

    let mut did_replace = true;
    while did_replace {
        (text, scratch_buffer) = str_replace_opt_ext(
            text,
            finder!("@@@@@@@@"),
            "@@@@",
            scratch_buffer,
            &mut did_replace,
        );
    }

    let result = text.split("@@@@").map(|s| s.to_string()).collect();

    text.clear();
    // scratch_buffer is already empty

    *scratch_buffers.0 = text;
    *scratch_buffers.1 = scratch_buffer;

    result
}

pub fn split_into_tokens(text: &str) -> Vec<String> {
    if cfg!(feature = "optimized-str") {
        split_into_tokens_corasick(text)
    } else {
        split_into_tokens_naive(text)
    }
}

#[doc(hidden)] /* only public for benchmarking */
pub fn split_into_tokens_naive(text: &str) -> Vec<String> {
    let text = text
        .replace("|", "||ææææ||")
        .replace("\n", "||")
        .replace(" ", "||");

    let symbols = [
        '.', ',', ';', ':', '?', '!', '-', '_', '/', '\\', '(', ')', '[', ']', '{', '}', '*', '#',
        '@', '&', '=', '+', '%', '~', '$', '^', '<', '>', '"', '\'', '´', '`', '¸', '˛', '’', '¤',
        '₳', '฿', '₵', '¢', '₡', '₢', '₫', '₯', '֏', '₠', '€', 'ƒ', '₣', '₲', '₴', '₭', '₺', '₾',
        'ℳ', '₥', '₦', '₧', '₱', '₰', '£', '៛', '₽', '₹', '₨', '₪', '৳', '₸', '₮', '₩', '¥', '§',
        '‖', '¦', '⟨', '⟩', '–', '—', '¯', '»', '«', '”', '÷', '×', '′', '″', '‴', '¡', '¿', '©',
        '℗', '®', '℠', '™',
    ];

    let mut text = text;
    for symbol in symbols {
        let sym_str = format!("||{}||", symbol);
        text = text.replace(symbol, &sym_str);
    }

    let text = text.replace("[||||[", "[[").replace("]||||]", "]]");
    let text = text.replace("{||||{", "{{").replace("}||||}", "}}");
    let text = text
        .replace("<||||!||||-||||-||", "||<!--||")
        .replace("||-||||-||||>", "||-->||");

    let mut text = text;
    while text.contains("||||") {
        text = text.replace("||||", "||");
    }

    text.split("||")
        .filter(|&s| !s.is_empty())
        .map(|w| {
            if w == "ææææ" {
                "|".to_string()
            } else {
                w.to_string()
            }
        })
        .collect()
}

#[doc(hidden)] /* only public for benchmarking */
pub fn split_into_tokens_corasick(text: &str) -> Vec<String> {
    // used to determine whether a match is a separator or a symbol
    const FIRST_SYMBOL: PatternID = PatternID::new_unchecked(2);
    const PATTERNS: &[&str] = &[
        /* separators --> */ " ", "\n", /* match composite symbols first --> */ "<!--",
        "-->", "[[", "]]", "{{", "}}", /* then match single character symbols --> */ "|", ".",
        ",", ";", ":", "?", "!", "-", "_", "/", "\\", "(", ")", "[", "]", "{", "}", "*", "#", "@",
        "&", "=", "+", "%", "~", "$", "^", "<", ">", "\"", "'", "´", "`", "¸", "˛", "’", "¤", "₳",
        "฿", "₵", "¢", "₡", "₢", "₫", "₯", "֏", "₠", "€", "ƒ", "₣", "₲", "₴", "₭", "₺", "₾", "ℳ",
        "₥", "₦", "₧", "₱", "₰", "£", "៛", "₽", "₹", "₨", "₪", "৳", "₸", "₮", "₩", "¥", "§", "‖",
        "¦", "⟨", "⟩", "–", "—", "¯", "»", "«", "”", "÷", "×", "′", "″", "‴", "¡", "¿", "©", "℗",
        "®", "℠", "™",
    ];
    const _: () = {
        let first_symbol = PATTERNS[FIRST_SYMBOL.as_usize()];
        assert!(const_str_equals(first_symbol, "<!--"));
    };

    static AHO_CORASICK: LazyLock<AhoCorasick> = LazyLock::new(|| {
        let mut builder = AhoCorasickBuilder::new();
        builder.match_kind(aho_corasick::MatchKind::LeftmostFirst); /* assign priority by order in pattern slice */
        // builder.kind(Some(aho_corasick::AhoCorasickKind::DFA)); // test if it's faster
        let aho_corasick = builder.build(PATTERNS).unwrap();
        tracing::debug!(
            "built aho-corasick successfully, kind: {:?}",
            aho_corasick.kind()
        );
        aho_corasick
    });

    let mut result = Vec::new();

    let mut last_end = 0;
    for m in AHO_CORASICK.find_iter(text) {
        let start = m.start();
        let end = m.end();

        // check if there is text between the last match and the current match
        if start > last_end {
            // collect text between symbols/separators (i.e. words)
            let token = text[last_end..start].to_string();
            result.push(token);
        }

        let token = &text[start..end];
        // ignore separators
        if m.pattern() >= FIRST_SYMBOL {
            // collect symbols
            result.push(token.to_string());
        }

        last_end = end;
    }

    if last_end < text.len() {
        // collect remaining text (last word)
        let token = text[last_end..].to_string();
        result.push(token);
    }

    result
}

use std::{collections::HashMap, hash::Hash, ops::Range, sync::LazyLock};

use crate::{
    algorithm::{Analysis, RevisionPointer, WordPointer},
    dump_parser::Sha1Hash,
};

pub fn compute_avg_word_freq(token_list: &[Token], interner: &mut Interner<String>) -> f64 {
    let mut counter: HashMap<Token, u64> = HashMap::new();

    for token in token_list.iter() {
        let count = counter.get_mut(token);
        if let Some(count) = count {
            *count += 1;
        } else {
            counter.insert(*token, 1);
        }
    }

    let remove_list = [
        "<", ">", "tr", "td", "[", "]", "\"", "*", "==", "{", "}", "|", "-",
    ];

    for token in remove_list {
        let token = interner.intern(token.to_string());
        counter.remove(&token);
    }

    let sum: u64 = counter.values().sum();
    let count = counter.len();

    if count > 0 {
        sum as f64 / count as f64
    } else {
        0.0
    }
}

fn trim_end_in_place(s: &mut String) {
    let trimmed = s.trim_end();
    s.truncate(trimmed.len());
}

fn trim_start_in_place(s: &mut String) {
    let trimmed = s.trim_start();
    s.replace_range(..(s.len() - trimmed.len()), "");
}

pub fn trim_in_place(mut input: String) -> String {
    trim_end_in_place(&mut input);
    trim_start_in_place(&mut input);
    input
}

pub fn iterate_revision_tokens<'a>(
    analysis: &'a Analysis,
    revision: &RevisionPointer,
) -> impl Iterator<Item = &'a WordPointer> + 'a {
    let revision = &analysis[revision];

    revision
        .paragraphs_ordered
        .iter()
        .flat_map(move |paragraph| {
            analysis[paragraph]
                .sentences_ordered
                .iter()
                .flat_map(move |sentence| analysis[sentence].words_ordered.iter())
        })
}

pub fn to_lowercase(input: &str) -> String {
    if cfg!(feature = "optimized-str") {
        to_lowercase_opt(input)
    } else {
        // for languages that have very little unicode (so basically: english), this is probably faster
        input.to_lowercase()
    }
}

#[doc(hidden)] /* only public for benchmarking */
pub fn to_lowercase_opt(input: &str) -> String {
    let mut result = String::with_capacity(input.len());
    for c in input.chars() {
        match unicode_case_mapping::to_lowercase(c) {
            [0, 0] => result.push(c),
            [l, 0] => result.push(char::from_u32(l).unwrap()),
            [l, l2] => {
                result.push(char::from_u32(l).unwrap());
                result.push(char::from_u32(l2).unwrap());
            }
        }
    }
    result
}

pub enum ChangeTag {
    Equal,
    Insert,
    Delete,
}

pub fn imara_diff(
    old: &[Token],
    new: &[Token],
    total_interned_tokens: u32,
) -> Vec<Option<(ChangeTag, Token)>> {
    let mut result = Vec::new();

    let mut last_old_pos = 0;
    imara_diff::diff_with_tokens(
        Algorithm::Histogram,
        old,
        new,
        total_interned_tokens,
        |before: Range<u32>, after: Range<u32>| {
            if before.start > last_old_pos {
                for token in &old[last_old_pos as usize..before.start as usize] {
                    result.push(Some((ChangeTag::Equal, *token)));
                }
            }
            last_old_pos = before.end;

            for token in &new[after.start as usize..after.end as usize] {
                result.push(Some((ChangeTag::Insert, *token)));
            }

            for token in &old[before.start as usize..before.end as usize] {
                result.push(Some((ChangeTag::Delete, *token)));
            }
        },
    );

    if last_old_pos < old.len() as u32 {
        for token in &old[last_old_pos as usize..] {
            result.push(Some((ChangeTag::Equal, *token)));
        }
    }

    result
}

#[cfg(feature = "python-diff")]
pub fn python_diff(old: &[Token], new: &[Token], interner: &mut Interner<String>) -> Vec<Option<(ChangeTag, Token)>> {
    use pyo3::{
        prelude::*,
        types::{PyList, PyString},
    };

    Python::with_gil(|py| {
        let builtins = py.import_bound("builtins").unwrap();
        let difflib = py.import_bound("difflib").unwrap();
        let differ = difflib.getattr("Differ").unwrap().call0().unwrap();

        let old = PyList::new_bound(py, old.iter().map(|&token| &interner[token]));
        let new = PyList::new_bound(py, new.iter().map(|&token| &interner[token]));

        let diff = differ.call_method1("compare", (old, new)).unwrap();
        let diff = builtins
            .call_method1("list", (diff,))
            .unwrap()
            .downcast_into::<PyList>()
            .unwrap();

        let mut result = Vec::new();
        for item in diff.iter() {
            let diff_item = item.downcast::<PyString>().unwrap();
            let diff_item = diff_item.to_str().unwrap();

            let tag = match diff_item.chars().next().unwrap() {
                ' ' => Some(ChangeTag::Equal),
                '+' => Some(ChangeTag::Insert),
                '-' => Some(ChangeTag::Delete),
                _ => None, /* apparently it can be '?' for example; I have no idea how diff algorithms work */
            };

            if let Some(tag) = tag {
                let value = interner.intern(diff_item[2..].to_string());
                result.push(Some((tag, value)));
            }
        }

        result
    })
}

#[cfg(not(feature = "python-diff"))]
pub fn python_diff(_old: &[Token], _new: &[Token], _interner: &mut Interner<String>) -> Vec<Option<(ChangeTag, Token)>> {
    panic!("python-diff feature is not enabled");
}

#[cfg(test)]
mod tests {
    use super::*;

    // standard unit tests

    #[test]
    fn test_split_into_paragraphs() {
        let text = "Hello\n\nWorld!";
        let result = split_into_paragraphs_naive(text);
        assert_eq!(result, vec!["Hello", "World!"]);
    }

    proptest! {
        #![proptest_config(ProptestConfig {
            cases: 100000,
            ..ProptestConfig::default()
        })]
        #[test]
        fn compare_split_into_paragraphs_optimized(input in "(\n|\r|\\||-|table|tr|<|>|\\}|\\{|.|.|.|.|.)*") {
            let mut scratch_buffers = (String::new(), String::new());

            let expected = split_into_paragraphs_naive(&input);
            let result_optimized = split_into_paragraphs_optimized(&input, (&mut scratch_buffers.0, &mut scratch_buffers.1));

            prop_assert!(scratch_buffers.0.is_empty());
            prop_assert!(scratch_buffers.1.is_empty());
            prop_assert_eq!(expected, result_optimized);
        }
    }

    proptest! {
        #![proptest_config(ProptestConfig {
            cases: 100000,
            ..ProptestConfig::default()
        })]
        #[test]
        fn compare_split_into_sentences_optimized(input in "(http|\\.|=|\\s|ref|-|!|/|:|\n|\r|\\?|;|\t|\\||.|.|.|.|.)*") {
            let mut scratch_buffers = (String::new(), String::new());

            let expected = split_into_sentences_naive(&input);
            let result_optimized = split_into_sentences_optimized(&input, (&mut scratch_buffers.0, &mut scratch_buffers.1));

            prop_assert!(scratch_buffers.0.is_empty());
            prop_assert!(scratch_buffers.1.is_empty());
            prop_assert_eq!(expected, result_optimized);
        }
    }

    proptest! {
        #![proptest_config(ProptestConfig {
            cases: 100000,
            ..ProptestConfig::default()
        })]
        #[test]
        fn compare_split_into_tokens_optimized(input in "(\n| |!|<|>|-|\\[|\\]|\\{|\\}|\\?|:|ℳ|֏|™|.|.|.|.|.)*") {
            let expected = split_into_tokens_naive(&input);
            let result_corasick = split_into_tokens_corasick(&input);

            prop_assert_eq!(expected, result_corasick);
        }
    }

    // comparing with Python implementation

    use crate::test_support::prelude::*;

    fn call_split_fn_py(py: Python<'_>, input: &str, fn_name: &str) -> Vec<String> {
        let builtins = py.import_bound("builtins").unwrap();
        let split_fn = py
            .import_bound("WikiWho.utils")
            .unwrap()
            .getattr(fn_name)
            .unwrap();

        let result_iterator = split_fn.call1((input,)).unwrap();
        builtins
            .call_method1("list", (result_iterator,))
            .unwrap()
            .extract::<Vec<String>>()
            .unwrap()
    }

    proptest! {
        #![proptest_config(ProptestConfig {
            cases: 10000,
            ..ProptestConfig::default()
        })]
        #[test]
        fn compare_split_into_paragraphs_python(input in ".*") {
            with_gil!(py, {
                let result_rust = crate::utils::split_into_paragraphs_naive(&input);
                let result_py = call_split_fn_py(py, &input, "split_into_paragraphs");

                prop_assert_eq!(result_rust, result_py);
            })
        }
    }

    proptest! {
        #![proptest_config(ProptestConfig {
            cases: 10000,
            ..ProptestConfig::default()
        })]
        #[test]
        fn compare_split_into_sentences_python(input in ".*") {
            with_gil!(py, {
                let result_rust = crate::utils::split_into_sentences_naive(&input);
                let result_py = call_split_fn_py(py, &input, "split_into_sentences");

                prop_assert_eq!(result_rust, result_py);
            })
        }
    }

    proptest! {
        #![proptest_config(ProptestConfig {
            cases: 10000,
            ..ProptestConfig::default()
        })]
        #[test]
        fn compare_split_into_tokens_python(input in ".*") {
            with_gil!(py, {
                let result_rust = crate::utils::split_into_tokens_naive(&input);
                let result_py = call_split_fn_py(py, &input, "split_into_tokens");

                prop_assert_eq!(result_rust, result_py);
            })
        }
    }

    // individual test cases found by proptest for closer inspection
    #[test]
    fn test_case_1() {
        Python::with_gil(|py| {
            let tokens_rust = crate::utils::split_into_tokens("®\u{2000}￼");
            let tokens_py = call_split_fn_py(py, "®\u{2000}￼", "split_into_tokens");

            assert_eq!(tokens_rust, tokens_py);
            assert_eq!(tokens_rust, vec!["®", "\u{2000}￼"]); // this should be what Python produces
        })
    }
}
