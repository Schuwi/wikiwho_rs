## Optimization TODO
Hotspots:
- `utils::split_into_sentences` -> `Regex::new` :facepalming: ✅
- `utils::split_into_tokens` -> `replace`✅, `format!`✅, `String::drop` ✅
- `utils::split_into_paragraphs_naive` -> `replace` ✅ (16%)
  -> try `aho-corasick` crate✅
- `Analysis::analyse_words_in_sentences` -> `iter().find(...)` (12 - 19%)
- `RevisionData::from_revision` -> `text.to_lowercase()` (18%)✅
  -> try `unicode-case-mapping` crate✅
- try `imara-diff` crate for diffing