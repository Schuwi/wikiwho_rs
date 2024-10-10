use aho_corasick::{AhoCorasick, AhoCorasickBuilder, PatternID};

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

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum RevisionHash {
    Sha1(Sha1Hash),
    Blake3(blake3::Hash),
}

pub fn split_into_paragraphs(text: &str) -> Vec<String> {
    if cfg!(feature = "optimized_str") {
        split_into_paragraphs_corasick(text)
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
pub fn split_into_paragraphs_corasick(text: &str) -> Vec<String> {
    const FIRST_SEPARATOR: usize = 0;
    const FIRST_PARAGRAPH_BEGINNING: usize = 8;
    const FIRST_PARAGRAPH_ENDING: usize = 14;
    const FIRST_REPLACEMENT: usize = 17;
    const PATTERNS_LEN: usize = PATTERNS.len();

    const PATTERNS: &[&str] = &[
        /* separators (order is important!) --> */
        "\n\n",
        "\n\r\n",
        "\n\r",
        "\r\r\n",
        "\r\r",
        "\r\n\n",
        "\r\n\r\n",
        "\r\n\r",
        /* paragraph beginnings --> */
        "<table>",
        "<tr>",
        "{|",
        "|-\n",
        "|-\r\n",
        "|-\r",
        /* paragraph endings --> */
        "</table>",
        "</tr>",
        "|}",
        /* replacements --> */
        "\r\n",
        "\r",
    ];

    const _: () = {
        assert!(const_str_equals(PATTERNS[FIRST_SEPARATOR], "\n\n"));
        assert!(const_str_equals(
            PATTERNS[FIRST_PARAGRAPH_BEGINNING],
            "<table>"
        ));
        assert!(const_str_equals(
            PATTERNS[FIRST_PARAGRAPH_ENDING],
            "</table>"
        ));
        assert!(const_str_equals(PATTERNS[FIRST_REPLACEMENT], "\r\n"));
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

    let mut current_paragraph = String::new();
    let mut last_end = 0;
    for m in AHO_CORASICK.find_iter(text) {
        let start = m.start();
        let end = m.end();

        // check if there is text between the last match and the current match
        if start > last_end {
            // collect text between separators (i.e. paragraphs)
            let paragraph_part = &text[last_end..start];
            current_paragraph.push_str(paragraph_part);
        }

        let pattern_id = m.pattern().as_usize();
        match pattern_id {
            FIRST_SEPARATOR..FIRST_PARAGRAPH_BEGINNING => {
                // separator
                // - ends the previous paragraph
                // - starts a new paragraph
                // - does not contain any text
                result.push(current_paragraph.clone());
                current_paragraph.clear();
            }
            FIRST_PARAGRAPH_BEGINNING..FIRST_PARAGRAPH_ENDING => {
                // paragraph beginning marker
                // - ends the previous paragraph
                // - starts a new paragraph
                // - will itself be part of the new paragraph
                result.push(current_paragraph.clone());
                current_paragraph.clear();

                current_paragraph.push_str(&text[start..end]);
            }
            FIRST_PARAGRAPH_ENDING..FIRST_REPLACEMENT => {
                // paragraph ending marker
                // - is itself part of the current paragraph
                // - ends the current paragraph
                // - starts a new paragraph
                current_paragraph.push_str(&text[start..end]);

                result.push(current_paragraph.clone());
                current_paragraph.clear();
            }
            FIRST_REPLACEMENT..PATTERNS_LEN => {
                // replacement
                // - replace with '\n'
                current_paragraph.push_str("\n");
            }
            _ => unreachable!(),
        }

        last_end = end;
    }

    if last_end < text.len() {
        // collect remaining text
        let paragraph_part = &text[last_end..];
        current_paragraph.push_str(&paragraph_part);
    }

    // collect the last paragraph
    result.push(current_paragraph);

    result
}

use regex::Regex;

pub fn split_into_sentences(text: &str) -> Vec<String> {
    if cfg!(feature = "optimized_str") {
        split_into_sentences_full_regex(text)
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

#[doc(hidden)] /* only public for benchmarking */
pub fn split_into_sentences_full_regex(text: &str) -> Vec<String> {
    static REGEX: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"(?P<newline_dot>\n\.) |(?P<ending>[^\s\.=][^\s\.=][^\s\.=]\.|;|\?|!|:) |(?P<ending_alt>\n|\t|-->|/ref>)|(?P<beginning><!--|<ref)|(?P<url>http.*?://.*?[ \|<>\n\r])").unwrap()
    });

    fn maybe_push_sentence(
        last_end: usize,
        current_sentence: &mut String,
        result: &mut Vec<String>,
    ) {
        let is_first = last_end == 0;
        if is_first || !current_sentence.is_empty() {
            result.push(current_sentence.clone());
            current_sentence.clear();
        }
        // else: ignore empty sentences
    }

    let mut result = Vec::new();

    let mut current_sentence = String::new();
    let mut last_end = 0;
    for c in REGEX.captures_iter(text) {
        let total = c.get(0).unwrap();
        let start = total.start();
        let end = total.end();

        // check if there is text between the last match and the current match
        if start > last_end {
            // collect text between separators (i.e. sentences)
            let sentence_part = &text[last_end..start];
            current_sentence.push_str(sentence_part);
        }

        // sorted according to expected frequency of occurrence
        if let Some(m) = c.name("ending") {
            // sentence ending
            // - is itself part of the current sentence
            // - ends the current sentence
            // - starts a new sentence
            current_sentence.push_str(m.as_str());

            maybe_push_sentence(last_end, &mut current_sentence, &mut result);
        } else if let Some(m) = c.name("ending_alt") {
            // sentence ending alternative
            // - is itself part of the current sentence
            // - ends the current sentence
            // - starts a new sentence
            current_sentence.push_str(m.as_str());

            maybe_push_sentence(last_end, &mut current_sentence, &mut result);
        } else if let Some(m) = c.name("beginning") {
            // sentence beginning
            // - ends the previous sentence
            // - starts a new sentence
            // - will itself be part of the new sentence
            maybe_push_sentence(last_end, &mut current_sentence, &mut result);

            current_sentence.push_str(m.as_str());
        } else if let Some(m) = c.name("url") {
            // url
            // - ends the previous sentence
            // - is itself a separate sentence
            // - starts a new sentence
            maybe_push_sentence(last_end, &mut current_sentence, &mut result);

            result.push(m.as_str().to_string());
        } else if let Some(_) = c.name("newline_dot") {
            // newline dot
            // - '\n' is part of the current sentence
            // - ends the current sentence
            // - '.' is itself a separate sentence
            // - starts a new sentence
            current_sentence.push_str("\n");
            maybe_push_sentence(last_end, &mut current_sentence, &mut result);

            result.push(".".to_string());
        } else {
            unreachable!();
        }

        last_end = end;
    }

    if last_end < text.len() {
        // collect remaining text
        let sentence_part = &text[last_end..];
        current_sentence.push_str(&sentence_part);
    }
    // the last sentence may be empty
    result.push(current_sentence);

    // --> first and last sentence may be empty

    result
}

pub fn split_into_tokens(text: &str) -> Vec<String> {
    if cfg!(feature = "optimized_str") {
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
        "-->", "[[", "]]", "{{", "}}", /* then match single character symbols --> */ ".", ",",
        ";", ":", "?", "!", "-", "_", "/", "\\", "(", ")", "[", "]", "{", "}", "*", "#", "@", "&",
        "=", "+", "%", "~", "$", "^", "<", ">", "\"", "'", "´", "`", "¸", "˛", "’", "¤", "₳", "฿",
        "₵", "¢", "₡", "₢", "₫", "₯", "֏", "₠", "€", "ƒ", "₣", "₲", "₴", "₭", "₺", "₾", "ℳ", "₥",
        "₦", "₧", "₱", "₰", "£", "៛", "₽", "₹", "₨", "₪", "৳", "₸", "₮", "₩", "¥", "§", "‖", "¦",
        "⟨", "⟩", "–", "—", "¯", "»", "«", "”", "÷", "×", "′", "″", "‴", "¡", "¿", "©", "℗", "®",
        "℠", "™",
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

use std::{collections::HashMap, ops::Range, sync::LazyLock};

use crate::{
    algorithm::{Analysis, RevisionPointer, WordPointer},
    dump_parser::Sha1Hash,
};

pub fn compute_avg_word_freq<S: AsRef<str>>(token_list: &[S]) -> f64 {
    let mut counter: HashMap<String, u64> = HashMap::new();

    for token in token_list.iter().map(AsRef::as_ref) {
        let count = counter.get_mut(token);
        if let Some(count) = count {
            *count += 1;
        } else {
            counter.insert(token.to_string(), 1);
        }
    }

    let remove_list = [
        "<", ">", "tr", "td", "[", "]", "\"", "*", "==", "{", "}", "|", "-",
    ];

    for token in remove_list {
        counter.remove(token);
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

use similar::ChangeTag;

#[cfg(feature = "python-diff")]
pub fn python_diff<S: AsRef<str> + pyo3::ToPyObject>(
    old: &[S],
    new: &[S],
) -> Vec<Option<(ChangeTag, String)>> {
    use pyo3::{
        prelude::*,
        types::{PyList, PyString},
    };

    Python::with_gil(|py| {
        let builtins = py.import_bound("builtins").unwrap();
        let difflib = py.import_bound("difflib").unwrap();
        let differ = difflib.getattr("Differ").unwrap().call0().unwrap();

        let old = PyList::new_bound(py, old);
        let new = PyList::new_bound(py, new);

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
            let value = diff_item[2..].to_string();

            result.push(tag.map(|tag| (tag, value)));
        }

        result
    })
}

#[cfg(not(feature = "python-diff"))]
pub fn python_diff<S: AsRef<str>>(_old: &[S], _new: &[S]) -> Vec<Option<(ChangeTag, String)>> {
    panic!("python-diff feature is not enabled");
}

#[cfg(test)]
mod tests {
    use rand::{Rng, SeedableRng};

    use super::*;

    // standard unit tests

    fn generate_input_split_into_paragraphs(seed: u64) -> String {
        // generate inputs from fixed seeds
        let mut rng = rand_xoshiro::Xoshiro256PlusPlus::seed_from_u64(seed); /* define specific algorithm to ensure reproducibility */
        let mut input = String::new();
        for _ in 0..5000 {
            input.push(rng.gen_range(0..128) as u8 as char);
        }

        // add some expected values at random places
        const VALUES: [&str; 17] = [
            "\n", "\n\n", "\n\n\n", "\r\n", "\r", "\r\r", "\r\r\r", "\r\n\r\n", "\n\r\n", "\n\n\r",
            "<table>", "</table>", "<tr>", "</tr>", "{|", "|}", "|-\n",
        ];
        for _ in 0..400 {
            let pos = rng.gen_range(0..input.len());
            input.insert_str(pos, VALUES[rng.gen_range(0..VALUES.len())]);
        }

        input
    }

    #[test]
    fn test_split_into_paragraphs_naive() {
        let text = "Hello\n\nWorld!";
        let result = split_into_paragraphs_naive(text);
        assert_eq!(result, vec!["Hello", "World!"]);
    }

    #[test]
    fn test_split_into_paragraphs_corasick() {
        let text = "Hello\n\nWorld!";
        let result = split_into_paragraphs_corasick(text);
        assert_eq!(result, vec!["Hello", "World!"]);
    }

    #[test]
    fn test_split_into_paragraphs_corasick_long() {
        let text = "
            Hello
            World!
            <table>\r
            <tr>
            <td>\r\rTest</td>
            </tr>
            </table>
        ";
        let result_naive = split_into_paragraphs_naive(text);
        let result_corasick = split_into_paragraphs_corasick(text);
        assert_eq!(result_naive, result_corasick,);
    }

    #[test]
    fn test_split_into_paragraphs_corasick_random() {
        for seed in 0..5 {
            let text = generate_input_split_into_paragraphs(seed);
            let result = split_into_paragraphs_corasick(&text);
            let expected = split_into_paragraphs_naive(&text);
            assert_eq!(result, expected);
        }
    }

    proptest! {
        #![proptest_config(ProptestConfig {
            cases: 10000,
            ..ProptestConfig::default()
        })]
        #[test]
        fn compare_split_into_paragraphs_optimized(input in "(<tr>|</tr>|<table>|</table>|\r|\n|\\{|\\||-|.|.|.|.|.)*") {
            let expected = split_into_paragraphs_naive(&input);
            let result_corasick = split_into_paragraphs_corasick(&input);

            prop_assert_eq!(expected, result_corasick);
        }
    }

    // comparing with Python implementation

    use crate::test_support::prelude::*;

    fn call_split_fn_py(py: Python<'_>, input: &str, fn_name: &str) -> Vec<String> {
        let builtins = py.import_bound("builtins").unwrap();
        let split_fn = py
            .import_bound("WikiWho.utils")
            // .unwrap()
            // .getattr("utils")
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
