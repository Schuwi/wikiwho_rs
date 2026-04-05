// SPDX-License-Identifier: MPL-2.0
use aho_corasick::{AhoCorasick, AhoCorasickBuilder, PatternID};
use memchr::memmem;
use regex::Regex;
use std::{borrow::Cow, sync::LazyLock};

use crate::utils::SemanticSubstringIterExt;

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

#[doc(hidden)] /* only public for benchmarking */
pub fn split_into_paragraphs_optimized<'a>(
    text: &'a str,
    scratch_buffers: (&mut String, &mut String),
) -> Vec<Cow<'a, str>> {
    let orig_text = text;
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

    let result = text
        .split("\n\n")
        .reborrow_semantic_substrings(orig_text)
        .collect();

    let mut text = text;
    text.clear();
    // scratch_buffer is already empty

    *scratch_buffers.0 = text;
    *scratch_buffers.1 = scratch_buffer;

    result
}

#[doc(hidden)] /* only public for benchmarking */
pub fn split_into_sentences_optimized<'a>(
    text: &'a str,
    scratch_buffers: (&mut String, &mut String),
) -> Vec<Cow<'a, str>> {
    let orig_text = text;

    thread_local! {
        static REGEX_DOT: Regex = Regex::new(r"([^\s\.=][^\s\.=][^\s\.=]\.) ").unwrap();
        static REGEX_URL: Regex = Regex::new(r"(http.*?://.*?[ \|<>\n\r])").unwrap();
    }

    scratch_buffers.0.push_str(text);

    let (text, scratch_buffer) = (
        std::mem::take(scratch_buffers.0),
        std::mem::take(scratch_buffers.1),
    );

    let (text, scratch_buffer) = str_replace_opt(text, finder!("\n"), "\n@@@@", scratch_buffer);

    let (text, scratch_buffer) =
        REGEX_DOT.with(|regex_dot| regex_replace_opt(text, regex_dot, "$1@@@@", scratch_buffer));

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

    let (text, scratch_buffer) = REGEX_URL
        .with(|regex_url| regex_replace_opt(text, regex_url, "@@@@$1@@@@", scratch_buffer));

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

    let result = text
        .split("@@@@")
        .reborrow_semantic_substrings(orig_text)
        .collect();

    text.clear();
    // scratch_buffer is already empty

    *scratch_buffers.0 = text;
    *scratch_buffers.1 = scratch_buffer;

    result
}

#[doc(hidden)] /* only public for benchmarking */
#[cfg(feature = "optimized-str")]
pub fn split_into_tokens_corasick(text: &str) -> Vec<Cow<'_, str>> {
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
            let token = &text[last_end..start];
            result.push(Cow::Borrowed(token));
        }

        let token = &text[start..end];
        // ignore separators
        if m.pattern() >= FIRST_SYMBOL {
            // collect symbols
            result.push(Cow::Borrowed(token));
        }

        last_end = end;
    }

    if last_end < text.len() {
        // collect remaining text (last word)
        let token = &text[last_end..];
        result.push(Cow::Borrowed(token));
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    proptest! {
        #![proptest_config(ProptestConfig {
            cases: 100000,
            ..ProptestConfig::default()
        })]
        #[test]
        fn compare_split_into_paragraphs_optimized(input in "(\n|\r|\\||-|table|tr|<|>|\\}|\\{|.|.|.|.|.)*") {
            let mut scratch_buffers = (String::new(), String::new());

            let expected = crate::utils::split_into_paragraphs_naive(&input);
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

            let expected = crate::utils::split_into_sentences_naive(&input);
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
            let expected = crate::utils::split_into_tokens_naive(&input);
            let result_corasick = split_into_tokens_corasick(&input);

            prop_assert_eq!(expected, result_corasick);
        }
    }

    // individual test cases found by proptest for closer inspection
    #[test]
    fn test_case_1() {
        let input = "®\u{2000}￼";

        let expected = crate::utils::split_into_tokens_naive(&input);
        let result_corasick = split_into_tokens_corasick(&input);

        assert_eq!(result_corasick, expected);
    }
}
