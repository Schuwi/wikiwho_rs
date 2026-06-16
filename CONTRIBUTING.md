# Contributing to wikiwho

Contributions are welcome! Here are some ways you can help:

- **Testing**: Try the library with different Wikimedia projects, languages, and dump versions.
- **Benchmarking**: Assist in creating benchmarks to compare performance and accuracy.
- **Documentation**: Improve existing documentation or add new examples and guides.
- **Feature Development**: Help implement new features like resumable parsing or configuration options.
- **Parser Enhancements**: Work on separating the parser into its own crate or improving its capabilities.

By submitting a contribution, you agree that your code will be licensed under this project's license.

## Getting Started

- Fork the repository: [wikiwho_rs GitHub](https://github.com/Schuwi/wikiwho_rs)
- Create a new branch for your feature or bug fix.
- Add an entry under `## [Unreleased]` in [`CHANGELOG.md`](CHANGELOG.md) (Keep a Changelog format). CI enforces this; for changes that don't warrant an entry (CI, docs, refactors) a maintainer can apply the `skip-changelog` label.
- If your change alters the public API in a breaking way, bump `version` in `Cargo.toml` (for `0.x`, the minor field) — the `semver` CI job checks this.
- Submit a pull request with a clear description of your changes.

## Development Setup

The exact comparison tests call into the original Python WikiWho implementation to validate results, so a Python virtual environment must be active when running them. Without it, tests will fail with cryptic Python/pyo3 errors.

```sh
python -m venv venv
source venv/bin/activate   # on Windows: venv\Scripts\activate
pip install -r requirements.txt
cargo test --features python-diff
```

To control where large temporary IPC files are written, set `TMPDIR` before running:

```sh
TMPDIR=/path/with/space cargo test --features python-diff
```

## Testing and Validation

- **Exact comparison tests** (`algorithm_exact_tests.rs`): Compare the Rust implementation's results against the original Python WikiWho, token by token. These require the `python-diff` feature so that both implementations use the same diff algorithm. Run them with `cargo test --features python-diff`.
- **Statistical comparison tests** (`algorithm_statistic_tests.rs`): Ignored by default and require local benchmark data. Fetch the archived partial gold standard with `python3 tools/fetch_gold_standard.py`, place current Wikimedia dump shards into `dev-data/extra-dumps/`, then run with `cargo test gold_standard_precision_rust -- --ignored` or `cargo test --features python-diff divergence_rate_gold_standard_articles -- --ignored`. See `dev-data/README.md` for details. CI runs these against a committed cache of gold-standard article histories (the pure-Rust precision test on every PR; the python-diff baselines on push to `main`).
- **Temporary files**: Some tests use temporary files for IPC coordination between Rust and Python. These files can be large depending on the input dump. Their location follows `std::env::temp_dir()`, which can be controlled by setting the `TMPDIR` environment variable.
- **Test dump location**: Real-page tests read a reference dump; set `WIKIWHO_TEST_DUMP=/path/to/dump.xml.zst` to override the default path. If the dump is absent, those tests skip (with a `SKIP:` notice) instead of failing.
- **Community Feedback**: Seeking input from users testing with different languages and datasets.

### Continuous Integration

CI runs on GitHub Actions (`.github/workflows/`):

- **`ci.yml`** (every push to `main` and every PR): `rustfmt`, Clippy across feature combinations (`-D warnings`), `cargo test --lib` + doc-tests, docs with warnings as errors, an MSRV check (Rust 1.94.1), coverage via `cargo-llvm-cov` (uploaded to Codecov), `cargo package`, a **SemVer check** (`cargo-semver-checks` over `--all-features` vs the latest crates.io release), a **changelog check** (PRs must add an entry to `CHANGELOG.md`), and — the headline jobs — **deterministic parity against the reference Python WikiWho** (`algorithm_exact_tests`) and **accuracy against the paper's gold standard** (`algorithm_statistic_tests`). Pull requests run parity against a small committed dump subset and the pure-Rust gold-standard precision test; pushes to `main` additionally fetch the full dump for deeper real-page parity and run the python-diff gold-standard baselines.
  - The SemVer check enforces **continuous version bumping**: a PR that makes a breaking API change must bump `version` in `Cargo.toml` accordingly (for `0.x`, the minor field, e.g. `0.3.x` → `0.4.0`), or CI fails. Purely additive changes are not forced to bump.
- **`fuzz.yml`** (weekly + manual): randomized property-test fuzzing of Rust-vs-Python parity. A failure uploads the discovered `*.proptest-regressions` seed so it can be committed as a permanent regression.
- **`heavy.yml`** (manual only): big-history parity and the opt-in ~25 GB multithreaded parity test against the full dump.

Test data:

- The representative subset (`dewiktionary-20240901-ci-subset.xml.zst`, ~900 KB) is committed via Git LFS, so contributors and CI get it on clone — no download needed. Regenerate it from a full dump with `python3 tools/make_ci_subset.py`.
- The full 808 MB dump lives in the [`Schuwi/wikiwho-data`](https://github.com/Schuwi/wikiwho-data) release. Fetch it (checksum-verified) with `python3 tools/fetch_test_data.py --which full`.

## Project Status and Support

- **Current Maintainer**: Working independently with assistance from various tools and collaborations.
- **Versioning**: Follows semantic versioning. Expect potential breaking changes before reaching 1.0.0.
- **Updates**: Development is on-demand. Regular maintenance depends on community interest and contributions.

Maintainers cutting a release should see [`RELEASING.md`](RELEASING.md).
