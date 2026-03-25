mod common;

use common::prelude::*;

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
            let result_rust = wikiwho::utils::split_into_paragraphs_naive(&input);
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
            let result_rust = wikiwho::utils::split_into_sentences_naive(&input);
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
            let result_rust = wikiwho::utils::split_into_tokens_naive(&input);
            let result_py = call_split_fn_py(py, &input, "split_into_tokens");

            prop_assert_eq!(result_rust, result_py);
        })
    }
}

// individual test cases found by proptest for closer inspection
#[test]
fn test_case_1() {
    Python::with_gil(|py| {
        let tokens_rust = wikiwho::utils::split_into_tokens_naive("®\u{2000}￼");
        let tokens_py = call_split_fn_py(py, "®\u{2000}￼", "split_into_tokens");

        assert_eq!(tokens_rust, tokens_py);
        assert_eq!(tokens_rust, vec!["®", "\u{2000}￼"]); // this should be what Python produces
    })
}
