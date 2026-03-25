// SPDX-License-Identifier: MPL-2.0
use imara_diff::{
    intern::{Interner, Token},
    Algorithm,
};
use regex::Regex;

use std::fmt::Debug;

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
pub fn split_into_paragraphs(
    text: &str,
    #[allow(unused)] // Only used when `optimized-str` feature active
    scratch_buffers: (&mut String, &mut String),
) -> Vec<String> {
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

pub fn split_into_sentences(
    text: &str,
    #[allow(unused)] // Only used when `optimized-str` feature active
    scratch_buffers: (&mut String, &mut String),
) -> Vec<String> {
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

pub fn split_into_tokens(text: &str) -> Vec<String> {
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

pub fn to_lowercase(input: &str, #[allow(unused)] analysis_options: PageAnalysisOptions) -> String {
    #[cfg(feature = "optimized-lowercase")]
    {
        if analysis_options.optimize_non_ascii {
            to_lowercase_opt(input)
        } else {
            input.to_lowercase()
        }
    }
    #[cfg(not(feature = "optimized-lowercase"))]
    {
        // for languages that have very little unicode (so basically: english), this is probably faster
        input.to_lowercase()
    }
}

#[doc(hidden)] /* only public for benchmarking */
#[cfg(feature = "optimized-lowercase")]
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

use std::{collections::HashMap, hash::Hash, ops::Range, sync::LazyLock};

use crate::{
    algorithm::{PageAnalysis, PageAnalysisOptions, RevisionPointer, WordPointer},
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

pub(crate) enum ChangeTag {
    Equal,
    Insert,
    Delete,
}

pub(crate) fn imara_diff(
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
pub(crate) fn python_diff(
    old: &[Token],
    new: &[Token],
    interner: &mut Interner<String>,
) -> Vec<Option<(ChangeTag, Token)>> {
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
