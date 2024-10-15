## Optimization TODO
Hotspots:
- `utils::split_into_sentences` -> `Regex::new` :facepalming: ✅
- `utils::split_into_tokens` -> `replace`✅, `format!`✅, `String::drop` ✅
- `utils::split_into_paragraphs_naive` -> `replace` ✅ (16%)
  -> try `aho-corasick` crate✅
- `Analysis::analyse_words_in_sentences` -> `iter().find(...)` (12 - 19%)✅
  -> try `imara-diff` crate for diffing, use their string interner for `find` as well✅
- `RevisionData::from_revision` -> `text.to_lowercase()` (18%)✅
  -> try `unicode-case-mapping` crate✅

## Licensing
This project is primarily licensed under the Mozilla Public License 2.0.

However, parts of this project are derived from the
[original `WikiWho` python implementation](https://github.com/wikiwho/WikiWho/), which is licensed
under the MIT License. Thus for these parts of the project (as marked by the SPDX headers), the
MIT License applies additionally.\
This basically just means that copyright notice in LICENSE-MIT must be preserved.