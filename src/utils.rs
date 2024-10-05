use arraydeque::ArrayDeque;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum RevisionHash {
    Sha1(Sha1Hash),
    Blake3(blake3::Hash),
}

enum ReplacementItem<T: 'static> {
    Item(T),
    Replacement(&'static [T]),
}

impl<T: 'static + Clone> IntoIterator for ReplacementItem<T> {
    type Item = T;
    type IntoIter = ReplacementItemIterator<T>;

    fn into_iter(self) -> Self::IntoIter {
        ReplacementItemIterator::new(self)
    }
}

struct ReplacementItemIterator<T: 'static> {
    item: Option<ReplacementItem<T>>,
    index: usize,
}

impl<T: 'static> ReplacementItemIterator<T> {
    fn new(item: ReplacementItem<T>) -> Self {
        Self {
            item: Some(item),
            index: 0,
        }
    }
}

impl<T: 'static + Clone> Iterator for ReplacementItemIterator<T> {
    type Item = T;

    fn next(&mut self) -> Option<Self::Item> {
        match self.item.take() {
            Some(ReplacementItem::Item(item)) => Some(item),
            Some(ReplacementItem::Replacement(replacement)) => {
                if self.index < replacement.len() {
                    let yield_item = replacement[self.index].clone();
                    self.index += 1;
                    self.item = Some(ReplacementItem::Replacement(replacement));
                    Some(yield_item)
                } else {
                    self.item = None;
                    None
                }
            }
            None => None,
        }
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        match self.item {
            Some(ReplacementItem::Item(_)) => (1, Some(1)),
            Some(ReplacementItem::Replacement(replacement)) => {
                let remaining = replacement.len() - self.index;
                (remaining, Some(remaining))
            }
            None => (0, Some(0)),
        }
    }
}

impl<T: 'static + Clone> ExactSizeIterator for ReplacementItemIterator<T> {
    fn len(&self) -> usize {
        match self.item {
            Some(ReplacementItem::Item(_)) => 1,
            Some(ReplacementItem::Replacement(replacement)) => replacement.len() - self.index,
            None => 0,
        }
    }
}

impl<T: 'static + Clone> FusedIterator for ReplacementItemIterator<T> {}

struct ReplaceIterator<T: 'static, I, const N: usize> {
    inner: Fuse<I>,
    buffer: ArrayDeque<T, N, arraydeque::Wrapping>,

    needle: &'static [T],
    matched_items: usize,
    replacement: &'static [T],
}

impl<
        T: 'static + Default + Clone + Eq + std::fmt::Debug,
        I: Iterator<Item = T>,
        const N: usize,
    > FusedIterator for ReplaceIterator<T, I, N>
where
    I: Iterator<Item = T>,
{
}

impl<T: 'static + Default + Clone, I: Iterator<Item = T>, const N: usize> ReplaceIterator<T, I, N> {
    fn new(inner: I, needle: &'static [T], replacement: &'static [T]) -> Self {
        assert!(needle.len() == N);
        assert!(needle.len() > 0);
        Self {
            inner: inner.fuse(),
            buffer: ArrayDeque::new(),
            needle,
            matched_items: 0,
            replacement,
        }
    }

    fn size_hint_for_input_len(&self, input_len: usize) -> (usize, Option<usize>) {
        let net_replacement_diff = self.replacement.len() as isize - N as isize; /* minimum value: -N */

        // assume as many replacements as possible
        let replacements = input_len / N; /* flooring */
        let lower = isize::min(
            input_len as isize + net_replacement_diff * replacements as isize,
            input_len as isize,
        );
        let upper = isize::max(
            input_len as isize + net_replacement_diff * replacements as isize,
            input_len as isize,
        );

        // casting to usize is fine because the values are guaranteed to be non-negative
        // worst case: input_len + (input_len / N) * (-N) = 0
        (lower as usize, Some(upper as usize))
    }
}

impl<
        T: 'static + Default + Clone + Eq + std::fmt::Debug,
        I: Iterator<Item = T>,
        const N: usize,
    > Iterator for ReplaceIterator<T, I, N>
{
    type Item = ReplacementItem<T>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.matched_items == N {
            debug_assert_eq!(self.buffer.len(), N);

            // drop items that are being replaced and reset match counter
            self.matched_items = 0;
            self.buffer.clear();

            Some(ReplacementItem::Replacement(self.replacement))
        } else {
            let mut yield_item = None;

            while let Some(next_item) = self.inner.next() {
                if next_item == self.needle[self.matched_items] {
                    self.matched_items += 1;
                } else if next_item == self.needle[0] {
                    self.matched_items = 1;
                } else {
                    self.matched_items = 0;
                }

                debug_assert!(self.matched_items <= N);

                if let Some(got_through) =
                    self.buffer.push_back(next_item).map(ReplacementItem::Item)
                {
                    yield_item = Some(got_through);
                    break;
                } else if self.matched_items == N {
                    debug_assert_eq!(self.buffer.len(), N);

                    // drop items that are being replaced and reset match counter
                    self.matched_items = 0;
                    self.buffer.clear();

                    yield_item = Some(ReplacementItem::Replacement(self.replacement));
                    break;
                }
                debug_assert!(self.matched_items <= self.buffer.len());
            }

            if yield_item.is_none() {
                // iterator is exhausted, return remaining items
                self.matched_items = 0;
                yield_item = self.buffer.pop_front().map(ReplacementItem::Item)
            }

            debug_assert!(
                self.matched_items <= self.buffer.len(),
                "assertion failed: {} (matched_items) <= {} (buffer.len()); buffer: {:?}",
                self.matched_items,
                self.buffer.len(),
                self.buffer
            );

            yield_item
        }
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let (inner_lower, inner_upper) = self.inner.size_hint();

        let lower = self.size_hint_for_input_len(inner_lower);
        let upper = inner_upper.map(|upper| self.size_hint_for_input_len(upper));

        if let Some(upper) = upper {
            // min and max can probably be avoided by thinking about the problem more
            // but I am too mush brained to do that right now
            let lower_res = usize::min(lower.0, upper.0);
            let upper_res = usize::max(
                lower.1.expect("upper bound must be known"),
                upper.1.expect("upper bound must be known"),
            );
            (lower_res, Some(upper_res))
        } else {
            (lower.0, None)
        }
    }
}

trait MyIteratorExt<T: 'static> {
    fn replace_all<const N: usize>(
        self,
        needle: &'static [T; N],
        replacement: &'static [T],
    ) -> impl Iterator<Item = T>;
}

impl<T: 'static + Default + Clone + Eq + std::fmt::Debug, I: Iterator<Item = T>> MyIteratorExt<T>
    for I
{
    fn replace_all<const N: usize>(
        self,
        needle: &'static [T; N],
        replacement: &'static [T],
    ) -> impl Iterator<Item = T> {
        ReplaceIterator::<T, _, N>::new(self, needle, replacement).flatten()
    }
}

pub fn split_into_paragraphs(text: &str) -> Vec<String> {
    if cfg!(feature = "optimized_str") {
        split_into_paragraphs_iterator(text)
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

// almost 5 times worse performance than the naive implementation, whoops
#[doc(hidden)] /* only public for benchmarking */
pub fn split_into_paragraphs_iterator(text: &str) -> Vec<String> {
    let mut iterator = text
        .chars()
        .replace_all(&['\r', '\n'], &['\n'])
        .replace_all(&['\r'], &['\n'])
        .replace_all(
            &['<', 't', 'a', 'b', 'l', 'e', '>'],
            &['\n', '\n', '<', 't', 'a', 'b', 'l', 'e', '>'],
        )
        .replace_all(
            &['<', '/', 't', 'a', 'b', 'l', 'e', '>'],
            &['<', '/', 't', 'a', 'b', 'l', 'e', '>', '\n', '\n'],
        )
        .replace_all(&['<', 't', 'r', '>'], &['\n', '\n', '<', 't', 'r', '>'])
        .replace_all(
            &['<', '/', 't', 'r', '>'],
            &['<', '/', 't', 'r', '>', '\n', '\n'],
        )
        .replace_all(&['{', '|'], &['\n', '\n', '{', '|'])
        .replace_all(&['|', '}'], &['|', '}', '\n', '\n'])
        .replace_all(&['|', '-', '\n'], &['\n', '\n', '|', '-', '\n'])
        .replace_all(&['\n', '\n'], &['\u{0091}']); /* replace double newline with a special character */

    let mut result = Vec::new();
    let mut paragraph = String::new();

    while let Some(c) = iterator.next() {
        if c == '\u{0091}' {
            result.push(paragraph.clone());
            paragraph.clear();
        } else {
            paragraph.push(c);
        }
    }
    result.push(paragraph);

    result
}

use regex::Regex;

pub fn split_into_sentences(text: &str) -> Vec<String> {
    let regex_dot = Regex::new(r"([^\s\.=][^\s\.=][^\s\.=]\.) ").unwrap();
    let regex_url = Regex::new(r"(http.*?://.*?[ \|<>\n\r])").unwrap();

    let text = text.replace("\n", "\n@@@@");
    let text = regex_dot.replace_all(&text, "$1@@@@");
    let text = text.replace("; ", ";@@@@");
    let text = text.replace("? ", "?@@@@");
    let text = text.replace("! ", "!@@@@");
    let text = text.replace(": ", ":@@@@");
    let text = text.replace("\t", "\t@@@@");
    let text = text.replace("<!--", "@@@@<!--");
    let text = text.replace("-->", "-->@@@@");
    let text = text.replace("<ref", "@@@@<ref");
    let text = text.replace("/ref>", "/ref>@@@@");
    let text = regex_url.replace_all(&text, "@@@@$1@@@@");

    let mut text = text.into_owned();
    while text.contains("@@@@@@@@") {
        text = text.replace("@@@@@@@@", "@@@@");
    }

    text.split("@@@@").map(|s| s.to_string()).collect()
}

pub fn split_into_tokens(text: &str) -> Vec<String> {
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

use std::{
    collections::HashMap,
    iter::{Fuse, FusedIterator},
};

use crate::{
    algorithm::{Analysis, RevisionPointer, WordPointer},
    dump_parser::Sha1Hash,
};

pub fn compute_avg_word_freq<S: AsRef<str>>(token_list: &[S]) -> f32 {
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
        sum as f32 / count as f32
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
    fn test_replacement_item_iterator() {
        let item = ReplacementItem::Item(1);
        let mut iterator = item.into_iter();
        assert_eq!(iterator.next(), Some(1));
        assert_eq!(iterator.next(), None);

        let item = ReplacementItem::Replacement(&[1, 2, 3]);
        let mut iterator = item.into_iter();
        assert_eq!(iterator.next(), Some(1));
        assert_eq!(iterator.next(), Some(2));
        assert_eq!(iterator.next(), Some(3));
        assert_eq!(iterator.next(), None);
    }

    #[test]
    fn test_replace_iterator() {
        let data = vec!["a", "b", "c", "d", "e"];
        let needle = &["b", "c"];
        let replacement = &["x", "y", "z"];

        let mut result = Vec::new();
        let mut iterator = data.into_iter().replace_all(needle, replacement);
        while let Some(item) = iterator.next() {
            result.push(item);
        }

        assert_eq!(result, vec!["a", "x", "y", "z", "d", "e"]);
    }

    #[test]
    fn test_replace_iterator_complex() {
        let data = vec!["a", "a", "a", "b", "b", "b", "a", "a"];
        let needle = &["b", "b"];
        let replacement = &["x", "y", "z"];

        let mut result = Vec::new();
        let mut iterator = data.into_iter().replace_all(needle, replacement);
        while let Some(item) = iterator.next() {
            result.push(item);
        }

        assert_eq!(result, vec!["a", "a", "a", "x", "y", "z", "b", "a", "a"]);
    }

    #[test]
    fn test_replace_iterator_no_match() {
        let data = vec!["a", "b", "c", "d", "e"];
        let needle = &["x", "y"];
        let replacement = &["1", "2"];
        let result: Vec<_> = data.into_iter().replace_all(needle, replacement).collect();
        assert_eq!(result, vec!["a", "b", "c", "d", "e"]);
    }

    #[test]
    fn test_replace_iterator_partial_match() {
        let data = vec!["a", "b", "c", "d", "e"];
        let needle = &["c", "d", "e"];
        let replacement = &["1", "2"];
        let result: Vec<_> = data.into_iter().replace_all(needle, replacement).collect();
        assert_eq!(result, vec!["a", "b", "1", "2"]);
    }

    #[test]
    fn test_split_into_paragraphs_naive() {
        let text = "Hello\n\nWorld!";
        let result = split_into_paragraphs_naive(text);
        assert_eq!(result, vec!["Hello", "World!"]);
    }

    #[test]
    fn test_split_into_paragraphs_iterator() {
        let text = "Hello\n\nWorld!";
        let result = split_into_paragraphs_iterator(text);
        assert_eq!(result, vec!["Hello", "World!"]);
    }

    #[test]
    fn test_split_into_paragraphs_iterator_long() {
        let text = "
            Hello
            World!
            <table>\r
            <tr>
            <td>\r\rTest</td>
            </tr>
            </table>
        ";
        let result = split_into_paragraphs_iterator(text);
        assert_eq!(
            result,
            vec![
                "Hello",
                "World!",
                "<table>",
                "<tr>",
                "<td>Test</td>",
                "</tr>",
                "</table>"
            ]
        );
    }

    #[test]
    fn test_split_into_paragraphs_iterator_random() {
        for seed in 0..5 {
            let text = generate_input_split_into_paragraphs(seed);
            let result = split_into_paragraphs_iterator(&text);
            let expected = split_into_paragraphs_naive(&text);
            assert_eq!(result, expected);
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
                let result_rust = crate::utils::split_into_paragraphs(&input);
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
                let result_rust = crate::utils::split_into_sentences(&input);
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
                let result_rust = crate::utils::split_into_tokens(&input);
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
