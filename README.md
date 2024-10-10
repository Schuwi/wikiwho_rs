## Optimization TODO
Hotspots:
- `utils::split_into_sentences` -> `Regex::new` :facepalming: ✅
- `utils::split_into_tokens` -> `replace`, `format!`, `String::drop` ⏳
- `utils::split_into_paragraphs_naive` -> `replace` ⏳
  -> try `aho-corasick` crate
- `Analysis::analyse_words_in_sentences` -> `iter().find(...)`
- `RevisionData::from_revision` -> `text.to_lowercase()`