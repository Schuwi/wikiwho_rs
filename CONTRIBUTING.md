# Contributing to wikiwho

Contributions are welcome! Here are some ways you can help:

- **Testing**: Try the library with different Wikimedia projects, languages, and dump versions.
- **Benchmarking**: Assist in creating benchmarks to compare performance and accuracy.
- **Documentation**: Improve existing documentation or add new examples and guides.
- **Feature Development**: Help implement new features like resumable parsing or configuration options.
- **Parser Enhancements**: Work on separating the parser into its own crate or improving its capabilities.

By submitting a contribution, you agree that your code will be licensed under this project's license.

## Getting Started

### Prerequisites

Install these **before cloning**:

- **[Git LFS](https://git-lfs.com/).** The committed test data — the representative dump subset and the gold-standard article cache — lives in Git LFS (`.gitattributes` routes every `*.zst` through it). Run `git lfs install` once, then clone. If you skip this, the `*.zst` files arrive as tiny text pointer files and tests fail with confusing decode errors; recover with `git lfs install && git lfs pull`.
- **A Rust toolchain** via [rustup](https://rustup.rs/). Everyday work uses `stable`. The MSRV is **1.94.1** (`rust-version` in `Cargo.toml`); add it with `rustup toolchain install 1.94.1` if you want to reproduce the MSRV gate locally (see [Reproducing the CI gates](#reproducing-the-ci-gates)).
- **Python 3** — only needed for the Rust-vs-Python parity tests (see [Development Setup](#development-setup)).

### Workflow

- Fork the repository ([wikiwho_rs on GitHub](https://github.com/Schuwi/wikiwho_rs)) and clone your fork.
- Create a new branch for your feature or bug fix.
- Add an entry under `## [Unreleased]` in [`CHANGELOG.md`](CHANGELOG.md) (Keep a Changelog format). CI enforces this; for changes that don't warrant an entry (CI, docs, refactors) a maintainer can apply the `skip-changelog` label.
- If your change alters the public API in a breaking way, bump `version` in `Cargo.toml` (for `0.x`, the minor field) — the `semver` CI job checks this.
- Submit a pull request with a clear description of your changes.

## Development Setup

Most of the suite is pure Rust and needs no setup — see [Running the tests](#running-the-tests). The Rust-vs-Python comparison tests, however, call into the original WikiWho implementation to validate results, so a Python virtual environment with that implementation installed must be active when running them. Without it they fail with cryptic Python/pyo3 errors.

```sh
python -m venv .venv
source .venv/bin/activate   # on Windows: .venv\Scripts\activate
pip install -r requirements.txt
cargo test --features python-diff,serde
```

> **`requirements.txt` is not an ordinary pip install.** It pins an *editable VCS checkout* of a [WikiWho fork](https://github.com/Schuwi/WikiWho) (forked to keep the `value` fields of sentence and paragraph objects for exact-equivalence tests):
>
> ```text
> -e git+https://github.com/Schuwi/WikiWho.git@<commit>#egg=WikiWho
> ```
>
> So `pip install` needs **`git` and network access** and will clone and build the fork — slower than a plain wheel download. This is exactly what CI does (`.venv` + `requirements.txt`). The fork diverges from upstream [`wikiwho/WikiWho`](https://github.com/wikiwho/WikiWho) only by that instrumentation and leaves the authorship algorithm unchanged, so parity against it is equivalent to parity against upstream.

To control where large temporary IPC files are written, set `TMPDIR` before running:

```sh
TMPDIR=/path/with/space cargo test --features python-diff,serde
```

## Running the tests

A bare `cargo test` is misleading here. The integration suites are **feature-gated at the file level**, so with the default features they compile to empty binaries that print `running 0 tests` and still exit `0` — a run that tested *nothing* looks exactly like a run that tested everything. Two things guard against that:

- **Run the canonical pure-Rust command**, the same one CI's `test` job uses (`.github/workflows/ci.yml`). This superset of the Python-free features exercises every unit and doc test — both the optimized and naive string paths, serde round-trips, and so on:

  ```sh
  cargo test --lib --features serde,cli,strict,optimized-str,optimized-lowercase
  cargo test --doc --features serde
  ```

- **Watch for `SKIP:` notices.** When a feature-gated suite compiles empty, the `skipped_suites` test binary reports a named, passing test that names the missing flag — e.g. `skipped_algorithm_exact_tests_enable_python_diff_and_serde`. Add `-- --nocapture` to see the full `SKIP:` line.

To actually run the feature-gated suites (the `python-diff` ones also need the Python venv from [Development Setup](#development-setup)):

| Suite | Needs | Command |
|---|---|---|
| `algorithm_exact_tests` (Rust-vs-Python parity) | `python-diff`, `serde` | `cargo test --features python-diff,serde --test algorithm_exact_tests` |
| `utils_comparisons` (tokenizer parity) | `python-diff` | `cargo test --features python-diff --test utils_comparisons` |
| `algorithm_statistic_tests` (gold-standard accuracy) | `serde` (+ data) | see [Testing and Validation](#testing-and-validation) |

> `tests/parser_tests.rs` is intentionally empty for now (tracked in [#6](https://github.com/Schuwi/wikiwho_rs/issues/6)); it likewise reports `running 0 tests`.

## Testing and Validation

- **Exact comparison tests** (`algorithm_exact_tests.rs`): Compare the Rust implementation's results against the original Python WikiWho, token by token. These require the `python-diff` and `serde` features (`python-diff` so both implementations use the same diff algorithm; `serde` for the fixture cache), so the whole suite is gated behind both. Run them with `cargo test --features python-diff,serde --test algorithm_exact_tests` (with the Python venv active; see [Development Setup](#development-setup)).
- **Statistical comparison tests** (`algorithm_statistic_tests.rs`): Gated behind `serde`, ignored by default, and require local benchmark data. Fetch the archived partial gold standard with `python3 tools/fetch_gold_standard.py`, place current Wikimedia dump shards into `dev-data/extra-dumps/`, then run with `cargo test --features serde --test algorithm_statistic_tests -- --ignored gold_standard_precision_rust` (pure Rust) or `cargo test --features python-diff,serde --test algorithm_statistic_tests -- --ignored divergence_rate_gold_standard_articles` (vs. Python). See [`dev-data/README.md`](dev-data/README.md) for details. CI runs these against a committed cache of gold-standard article histories (the pure-Rust precision test on every PR; the python-diff baselines on push to `main`).
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

### Reproducing the CI gates

Before pushing, you can reproduce every blocking PR gate locally. Each maps to a job in `.github/workflows/ci.yml`:

- **Format** (`fmt`): `cargo fmt --all --check`
- **Lints** (`clippy`) — run the same feature combinations CI does, each with `-D warnings`:

  ```sh
  for f in "--no-default-features" "" "--features serde" "--features cli" \
           "--features serde,cli,strict,optimized-str,optimized-lowercase"; do
    cargo clippy --all-targets $f -- -D warnings
  done
  ```

- **Tests** (`test`): the canonical command from [Running the tests](#running-the-tests).
- **MSRV** (`msrv`) — there is deliberately no `rust-toolchain.toml` (it would override the `stable` toolchain CI's other jobs use), so pass the pinned toolchain explicitly:

  ```sh
  rustup toolchain install 1.94.1   # once
  cargo +1.94.1 check --all-features
  ```

- **Docs** (`doc`): `RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --all-features`
- **SemVer** (`semver`) — a breaking public-API change must bump `version` in `Cargo.toml` (the minor field, for `0.x`):

  ```sh
  cargo install cargo-semver-checks   # once
  cargo semver-checks --all-features
  ```

- **Changelog** (`changelog`): the gate just greps the PR diff for a change to `CHANGELOG.md` — any edit under `## [Unreleased]` satisfies it (or a maintainer applies the `skip-changelog` label). It does **not** validate the content.

The headline parity and gold-standard jobs additionally need the Python venv and/or the LFS data; see [Development Setup](#development-setup) and [Running the tests](#running-the-tests).

## Project Status and Support

- **Maintainer**: [@Schuwi](https://github.com/Schuwi), working independently.
- **Versioning**: Follows semantic versioning. Expect potential breaking changes before reaching 1.0.0.
- **Updates**: Development is on-demand. Regular maintenance depends on community interest and contributions.

Maintainers cutting a release should see [`RELEASING.md`](RELEASING.md).
