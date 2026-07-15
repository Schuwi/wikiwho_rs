// SPDX-License-Identifier: MPL-2.0
use imara_diff::{Interner, Token};
use regex::Regex;

use std::{borrow::Cow, fmt::Debug, sync::Arc};

pub(crate) struct DebugStringEllipsis<'a>(pub &'a str, pub usize);

impl Debug for DebugStringEllipsis<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.0.len() > self.1 {
            let split = self
                .0
                .char_indices()
                .nth(self.1)
                .map(|(idx, _)| idx)
                .unwrap_or(self.0.len());
            write!(f, "{}...", &self.0[..split])
        } else {
            write!(f, "{}", self.0)
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) enum RevisionHash {
    Sha1(Sha1Hash),
    Blake3(blake3::Hash),
}

/// Split the input text into paragraphs.
///
/// # Arguments
///
/// * `text` - The input text to split.
/// * `scratch_buffers` - A tuple containing two scratch buffers to use for temporary storage.
///   They must be empty and will again be empty after the function returns.
///   They should be reused across multiple calls to this function.
pub fn split_into_paragraphs<'a>(
    text: &'a str,
    #[allow(unused)] // Only used when `optimized-str` feature active
    scratch_buffers: (&mut String, &mut String),
) -> Vec<Cow<'a, str>> {
    #[cfg(feature = "optimized-str")]
    {
        crate::optimized_str::split_into_paragraphs_optimized(text, scratch_buffers)
    }
    #[cfg(not(feature = "optimized-str"))]
    {
        split_into_paragraphs_naive(text)
    }
}

#[doc(hidden)] /* only public for benchmarking */
pub fn split_into_paragraphs_naive(text: &str) -> Vec<Cow<'_, str>> {
    let orig_text = text;

    let text = text.replace("\r\n", "\n").replace("\r", "\n");

    let text = text
        .replace("<table>", "\n\n<table>")
        .replace("</table>", "</table>\n\n");

    let text = text
        .replace("<tr>", "\n\n<tr>")
        .replace("</tr>", "</tr>\n\n");

    let text = text.replace("{|", "\n\n{|").replace("|}", "|}\n\n");
    let text = text.replace("|-\n", "\n\n|-\n");

    text.split("\n\n")
        .reborrow_semantic_substrings(orig_text)
        .collect()
}

pub fn split_into_sentences<'a>(
    text: &'a str,
    #[allow(unused)] // Only used when `optimized-str` feature active
    scratch_buffers: (&mut String, &mut String),
) -> Vec<Cow<'a, str>> {
    #[cfg(feature = "optimized-str")]
    {
        crate::optimized_str::split_into_sentences_optimized(text, scratch_buffers)
    }
    #[cfg(not(feature = "optimized-str"))]
    {
        split_into_sentences_naive(text)
    }
}

#[doc(hidden)] /* only public for benchmarking */
pub fn split_into_sentences_naive(text: &str) -> Vec<Cow<'_, str>> {
    thread_local! {
        static REGEX_DOT: Regex = Regex::new(r"([^\s\.=][^\s\.=][^\s\.=]\.) ").unwrap();
        static REGEX_URL: Regex = Regex::new(r"(http.*?://.*?[ \|<>\n\r])").unwrap();
    }

    let orig_text = text;

    let text = text.replace("\n", "\n@@@@");
    let text = REGEX_DOT.with(|regex_dot| regex_dot.replace_all(&text, "$1@@@@"));
    let text = text.replace("; ", ";@@@@");
    let text = text.replace("? ", "?@@@@");
    let text = text.replace("! ", "!@@@@");
    let text = text.replace(": ", ":@@@@");
    let text = text.replace("\t", "\t@@@@");
    let text = text.replace("<!--", "@@@@<!--");
    let text = text.replace("-->", "-->@@@@");
    let text = text.replace("<ref", "@@@@<ref");
    let text = text.replace("/ref>", "/ref>@@@@");
    let text = REGEX_URL.with(|regex_url| regex_url.replace_all(&text, "@@@@$1@@@@"));

    let mut text = text.into_owned();
    while text.contains("@@@@@@@@") {
        text = text.replace("@@@@@@@@", "@@@@");
    }

    text.split("@@@@")
        .reborrow_semantic_substrings(orig_text)
        .collect()
}

pub fn split_into_tokens(text: &str) -> Vec<Cow<'_, str>> {
    #[cfg(feature = "optimized-str")]
    {
        crate::optimized_str::split_into_tokens_corasick(text)
    }
    #[cfg(not(feature = "optimized-str"))]
    {
        split_into_tokens_naive(text)
    }
}

#[doc(hidden)] /* only public for benchmarking */
pub fn split_into_tokens_naive(text: &str) -> Vec<Cow<'_, str>> {
    let orig_text = text;

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

    let tokens = text
        .split("||")
        .filter(|&s| !s.is_empty())
        .map(|w| if w == "ææææ" { "|" } else { w })
        .reborrow_semantic_substrings(orig_text)
        .collect();

    tokens
}

pub fn to_lowercase(
    input: &str,
    #[allow(unused)] analysis_options: PageAnalysisOptions,
) -> (usize, String) {
    #[cfg(feature = "python-diff")]
    {
        // When comparing against the Python reference implementation, use Python's
        // Unicode tables for non-ASCII lowercasing so exact-equivalence tests stay
        // stable even when Rust and Python are built against different Unicode versions.
        if analysis_options.use_python_diff && !input.is_ascii() {
            return python_lowercase(input);
        }
    }

    #[cfg(feature = "optimized-lowercase")]
    {
        if analysis_options.optimize_non_ascii {
            to_lowercase_opt(input)
        } else {
            let lowercase = input.to_lowercase();
            (lowercase.chars().count(), lowercase)
        }
    }
    #[cfg(not(feature = "optimized-lowercase"))]
    {
        // for languages that have very little unicode (so basically: english), this is probably faster
        let lowercase = input.to_lowercase();
        (lowercase.chars().count(), lowercase)
    }
}

#[cfg(feature = "python-diff")]
fn python_lowercase(input: &str) -> (usize, String) {
    use pyo3::{prelude::*, types::PyString};

    Python::attach(|py| {
        let lowercase: String = PyString::new(py, input)
            .call_method0("lower")
            .unwrap()
            .extract()
            .unwrap();
        (lowercase.chars().count(), lowercase)
    })
}

#[doc(hidden)] /* only public for benchmarking */
#[cfg(feature = "optimized-lowercase")]
pub fn to_lowercase_opt(input: &str) -> (usize, String) {
    let mut result = String::with_capacity(input.len());
    let mut char_count = 0;

    for c in input.chars() {
        let lowercased = unicode_case_mapping::to_lowercase(c);

        if lowercased[1] > 0 {
            char_count += 2;
        } else {
            char_count += 1;
        }

        match lowercased {
            [0, 0] => result.push(c),
            [l, 0] => result.push(char::from_u32(l).unwrap()),
            [l, l2] => {
                result.push(char::from_u32(l).unwrap());
                result.push(char::from_u32(l2).unwrap());
            }
        }
    }

    (char_count, result)
}

use std::{collections::HashMap, hash::Hash, sync::LazyLock};

use crate::{
    algorithm::{ArcSubstring, PageAnalysis, PageAnalysisOptions, RevisionPointer, WordPointer},
    dump_parser::Sha1Hash,
};

pub fn compute_avg_word_freq(token_list: &[Token], interner: &mut Interner<ArcSubstring>) -> f64 {
    static REMOVE_LIST: LazyLock<Vec<ArcSubstring>> = LazyLock::new(|| {
        let remove_list = [
            "<", ">", "tr", "td", "[", "]", "\"", "*", "==", "{", "}", "|", "-",
        ];

        remove_list
            .iter()
            .map(|t_str| ArcSubstring::new_source(Arc::new(t_str.to_string())))
            .collect()
    });

    let mut counter: HashMap<Token, u64> = HashMap::new();

    for token in token_list {
        counter
            .entry(*token)
            .and_modify(|count| *count += 1)
            .or_insert(1);
    }

    for token in REMOVE_LIST.iter() {
        let token = interner.intern(token.clone());
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

pub fn trim_in_place(input: Cow<'_, str>) -> Cow<'_, str> {
    match input {
        Cow::Owned(mut owned_str) => {
            trim_end_in_place(&mut owned_str);
            trim_start_in_place(&mut owned_str);
            Cow::Owned(owned_str)
        }
        Cow::Borrowed(str_slice) => Cow::Borrowed(str_slice.trim()),
    }
}

/// Iterates over the tokens (≈ words) present in a revision, in reading order.
///
/// Given a [`RevisionPointer`] into `analysis`, this yields the tokens that are
/// *present* in that revision — in paragraph → sentence → word order, i.e. the
/// order in which they appear in the article text. Tokens that had been deleted by
/// this revision are not part of its structure and are therefore not yielded; to
/// inspect deletions, look at the add/delete history on the individual
/// [`WordAnalysis`](crate::algorithm::WordAnalysis) instead.
///
/// This is the primary way to consume analysis results. Pass
/// [`PageAnalysis::current_revision`] to walk the latest (non-spam) revision, or
/// any [`RevisionPointer`] from [`PageAnalysis::ordered_revisions`] for a
/// historical one, then index into `analysis` with each yielded [`WordPointer`]
/// (e.g. `&analysis[word]`) to read its authorship data, such as the
/// [`origin_revision`](crate::algorithm::WordAnalysis::origin_revision).
pub fn iterate_revision_tokens<'a>(
    analysis: &'a PageAnalysis,
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

pub trait SemanticSubstringIterExt<'b, I: Iterator<Item = &'b str> + 'b> {
    fn reborrow_semantic_substrings<'a: 'b>(
        self,
        source_str: &'a str,
    ) -> std::iter::Map<I, impl FnMut(&'b str) -> Cow<'a, str>>;
}

impl<'b, I> SemanticSubstringIterExt<'b, I> for I
where
    I: Iterator<Item = &'b str> + 'b,
{
    // parts should be ordered, non-overlapping substrings of `source_str` (by value),
    // otherwise falls back to allocation (String) for substrings not found in source
    fn reborrow_semantic_substrings<'a: 'b>(
        self,
        source_str: &'a str,
    ) -> std::iter::Map<I, impl FnMut(&'b str) -> Cow<'a, str>> {
        let mut remaining_source_str = source_str;

        self.map(move |part| {
            if let Some(index) = remaining_source_str.find(part) {
                let borrowed_part = &remaining_source_str[index..index + part.len()];
                remaining_source_str = &remaining_source_str[index + part.len()..];
                Cow::Borrowed(borrowed_part)
            } else {
                Cow::Owned(part.to_string())
            }
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ChangeTag {
    Equal,
    Insert,
    Delete,
}

pub(crate) fn difflib_diff(old: &[Token], new: &[Token]) -> Vec<Option<(ChangeTag, Token)>> {
    crate::difflib::compare(old, new)
        .into_iter()
        .map(|(tag, token)| Some((tag, *token)))
        .collect()
}

#[cfg(feature = "python-diff")]
pub(crate) fn python_diff(
    old: &[Token],
    new: &[Token],
    interner: &mut Interner<ArcSubstring>,
) -> Vec<Option<(ChangeTag, Token)>> {
    use pyo3::{
        prelude::*,
        types::{PyList, PyString},
    };

    Python::attach(|py| {
        let builtins = py.import("builtins").unwrap();
        let difflib = py.import("difflib").unwrap();
        let differ = difflib.getattr("Differ").unwrap().call0().unwrap();

        // we can't just use the token indices converted to string instead of the literal text
        // if we want to reproduce the original behavior because the diff algorithm
        // is content-aware due to a "junk" metric
        let old = PyList::new(py, old.iter().map(|&token| interner[token].as_str())).unwrap();
        let new = PyList::new(py, new.iter().map(|&token| interner[token].as_str())).unwrap();

        let diff = differ.call_method1("compare", (old, new)).unwrap();
        let diff = builtins
            .call_method1("list", (diff,))
            .unwrap()
            .cast_into::<PyList>()
            .unwrap();

        let mut result = Vec::new();
        let mut temporary_source = Arc::new(String::new());
        for item in diff.iter() {
            let diff_item = item.cast::<PyString>().unwrap();
            let diff_item = diff_item.to_str().unwrap();

            let tag = match diff_item.chars().next().unwrap() {
                ' ' => Some(ChangeTag::Equal),
                '+' => Some(ChangeTag::Insert),
                '-' => Some(ChangeTag::Delete),
                _ => None, /* ignore '?' annotations which are just for intra-token diff visualization */
            };

            if let Some(tag) = tag {
                let literal_token_buf = Arc::make_mut(&mut temporary_source);
                literal_token_buf.clear();
                literal_token_buf.push_str(&diff_item[2..]);
                // all of these token literals should already be interned, so the `ArcSubstring`
                // and thus the `temporary_source.clone()` should be dropped once `intern` returns,
                // making "our" `temporary_source` the only copy again,
                // which means `make_mut` can reuse the allocation next iteration
                // (we need this workaround since we can't tell the interner to just look up a &str)
                let value = interner.intern(ArcSubstring::new_source(temporary_source.clone()));
                result.push(Some((tag, value)));
            }
        }

        result
    })
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
}
