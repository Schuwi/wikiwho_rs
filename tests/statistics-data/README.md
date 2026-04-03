# Statistical Test Data

`tests/algorithm_statistic_tests.rs` uses local-only benchmark data and is `#[ignore]` by default.

This directory is the public recipe for that setup. The actual benchmark inputs stay untracked.

## Layout

- `gold_standard.partial.newnames.csv`
  - Local cache of the normalized partial WikiWho gold-standard CSV used by the tests.
  - Not committed.
- `article-pages/`
  - Optional single-page article extracts such as `Apollo_11.xml` or `Apollo_11.xml.zst`.
  - Not committed.
- `article-cache/`
  - Auto-generated page caches written by the tests as `.json.zst`.
  - May be committed, but these files are Wikimedia-derived test data, not software.
  - See `article-cache/README.md` and `article-cache/ATTRIBUTION.md`.
- `extra-dumps/`
  - Current Wikimedia dump shards searched when a page is not present in `article-pages/` or `article-cache/`.
  - Not committed.

## Gold Standard Setup

Prepare the test copy with:

```sh
python3 tests/fetch_stat_test_data.py
```

Important provenance details:

- `gold_standard.partial.csv` is a manually recovered intermediate, extracted from pre-loaded HTML data in the Wayback snapshot of the Google Sheets edit page:
  - `https://web.archive.org/web/20190626204719/https://docs.google.com/spreadsheets/d/1Xvl1NXqFY_efvoZ9oj2xH86fSljLYpDNI1dt2YfISlk/edit?usp=sharing`
- `gold_standard.partial.newnames.csv` is derived from that recovered CSV by applying a few manual title updates so the article names match current Wikipedia titles.

The script can recover the partial CSV directly from the pinned Wayback HTML snapshot and then apply the title normalization. The HTML recovery output is treated as canonical for checksum purposes.

If you want to inspect or preserve the recovered pre-rename intermediate explicitly, write it out with:

```sh
python3 tests/fetch_stat_test_data.py --write-recovered-csv /path/to/gold_standard.partial.csv
```

The title normalization is:

- `Armenian Genocide` -> `Armenian genocide`
- `Bioglass` -> `Bioglass 45S5`
- `Communist Party of China` -> `Chinese Communist Party`

If you want to force a specific source, pass it explicitly. Both CSV and HTML are supported, as local files or URLs:

```sh
python3 tests/fetch_stat_test_data.py --input /path/to/gold_standard.partial.csv
python3 tests/fetch_stat_test_data.py --input /path/to/gold_standard_wayback.html
python3 tests/fetch_stat_test_data.py --input 'https://web.archive.org/web/20190626204719/https://docs.google.com/spreadsheets/d/1Xvl1NXqFY_efvoZ9oj2xH86fSljLYpDNI1dt2YfISlk/edit?usp=sharing'
```

Older manually extracted CSV intermediates may differ from the HTML recovery output in insignificant whitespace, so the canonical checksum below is based on the HTML recovery path.

The resulting normalized file must match this SHA-256:

```text
77e88847c1939523a57953ca54c5137e256b487660c7d88aecb70ac1327df083
```

The normalized gold-standard CSV is generated locally on demand and intentionally not redistributed by this repository.

## Dump Setup

Put current Wikimedia dump shards into `tests/statistics-data/extra-dumps/`.

Supported formats:

- `.xml`
- `.xml.bz2`
- `.xml.gz`
- `.xml.zst`
- `.xml.zstd`

Recompressing XML dump shards to zstd is strongly recommended because the page scan path is much faster on `.zst` files (e.g. `bzip2 -dc ....xml.bz2 | zstd -11 -T4 -o ....xml.zst`).

The tests also accept per-article extracts in `tests/statistics-data/article-pages/`. On the first successful lookup from a dump shard, the page is cached automatically into `tests/statistics-data/article-cache/<Article>.json.zst`.

## Committing `article-cache/`

Committed page caches in `article-cache/` should be limited to Wikimedia-derived article text fixtures.

- Treat these files as data, not as code covered by the repository's MPL/MIT software license.
- Keep `article-cache/README.md` and `article-cache/ATTRIBUTION.md` with any committed cache files.
- Update `article-cache/ATTRIBUTION.md` with one entry per committed cache file, including at least:
  - cache filename
  - article title
  - page URL
  - history URL
  - note that the content was transformed from Wikimedia dump XML into serialized `.json.zst`
- Dump shard filenames or dates are useful provenance, but they are optional rather than required for attribution.

Wikimedia text dumps are generally reusable under CC BY-SA 4.0 and GFDL, with attribution and share-alike requirements. Use page or history URLs for attribution metadata.

## Running

Pure Rust precision:

```sh
cargo test gold_standard_precision_rust -- --ignored
```

Python baseline and divergence checks:

```sh
cargo test --features python-diff gold_standard_precision_python_diff -- --ignored
cargo test --features python-diff divergence_rate_gold_standard_articles -- --ignored
```
