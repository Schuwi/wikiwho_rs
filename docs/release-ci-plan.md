# Release CI plan (provenance & transparency)

**Status: plan / not implemented.** This designs an automated crates.io release for
`wikiwho` that satisfies two requirements:

1. **Authenticity** — the artifact on crates.io provably comes from this repo's CI, not
   from a maintainer's laptop.
2. **Correspondence** — anyone can verify the published crate matches a specific, public
   GitHub commit/tag.

## Mechanisms

### 1. Trusted Publishing (OIDC) — no long-lived token
crates.io supports **Trusted Publishing**: a one-time config on crates.io links the
`wikiwho` crate to a specific GitHub repo + workflow (+ optional environment). At publish
time, GitHub Actions mints a short-lived OIDC token that crates.io exchanges for an
ephemeral publish token. Result: **no `CARGO_REGISTRY_TOKEN` secret exists anywhere** to
leak or to publish from a laptop with. The crates.io "owners/publish" audit shows the
publish came from the GitHub Actions identity.

- One-time setup (maintainer, on crates.io): add a Trusted Publisher → repo
  `Schuwi/wikiwho_rs`, workflow `release.yml`, environment `release`.
- In CI: `permissions: id-token: write`, use the official action to obtain the token, then
  `cargo publish` (no token env).

### 2. Build provenance attestation (SLSA / Sigstore)
`actions/attest-build-provenance` signs an attestation tying the produced `.crate` to the
exact workflow run + commit, recorded in the public **Rekor transparency log** (Sigstore).
Anyone can later run:

```sh
gh attestation verify wikiwho-<version>.crate --repo Schuwi/wikiwho_rs
```

to confirm the file was built by this repo's release workflow at a given commit. Publish
the `.crate` + its attestation as **GitHub Release assets** so they're downloadable
alongside the source.

### 3. Correspondence to a GitHub commit
- The release is driven by a **git tag** `vX.Y.Z`; CI checks out that exact tag.
- `cargo package` embeds **`.cargo_vcs_info.json`** (the git commit SHA + clean-tree flag)
  inside the `.crate` when packaging from a clean git checkout. A user can download the
  crate from crates.io, extract it, read `.cargo_vcs_info.json`, and confirm it names the
  tagged commit — then diff the extracted sources against `git checkout vX.Y.Z`.
- CI asserts `Cargo.toml` `version` == tag and that the working tree is clean before
  publishing, so the `.crate` is reproducible from the tag.

## Proposed `release.yml`

Trigger: push tag `v*`. Outline:

```yaml
on:
  push:
    tags: ["v*"]
permissions:
  contents: write     # create the GitHub Release
  id-token: write     # OIDC: trusted publishing + attestation
  attestations: write
jobs:
  release:
    environment: release          # optional: required-reviewer gate on releases
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4           # checks out the tag
      - uses: dtolnay/rust-toolchain@stable
      - name: Version matches tag
        run: test "v$(cargo metadata --no-deps --format-version=1 | jq -r '.packages[0].version')" = "${GITHUB_REF_NAME}"
      - name: Verify CI is green for this commit
        run: gh run list --commit "$GITHUB_SHA" --workflow ci.yml ... # or use workflow_run gating
      - run: cargo package                  # produces target/package/wikiwho-<v>.crate
      - uses: actions/attest-build-provenance@v1
        with: { subject-path: "target/package/wikiwho-*.crate" }
      - name: Publish (Trusted Publishing)
        uses: rust-lang/crates-io-auth-action@v1   # mints OIDC token
        # then: cargo publish   (no CARGO_REGISTRY_TOKEN)
      - name: GitHub Release with .crate + checksums
        run: gh release create "$GITHUB_REF_NAME" target/package/wikiwho-*.crate --generate-notes
```

## Gating: only release tested code
The release must not publish code that failed parity/tests. Options:

- **Simplest:** the `release.yml` job re-runs the essential checks (`cargo test --lib`,
  `fmt`, `clippy`) before publishing. (Re-runs the cheap gates; the heavy parity already
  ran on the merge to `main` that the tag points at.)
- **Stricter:** require the `ci.yml` run for `$GITHUB_SHA` to be successful via a
  `workflow_run`/status check before the publish step proceeds.

## Release procedure (maintainer)
1. Bump `version` in `Cargo.toml`, update `CHANGELOG.md`, merge to `main` (CI green).
2. `git tag vX.Y.Z && git push origin vX.Y.Z`.
3. `release.yml` publishes to crates.io (Trusted Publishing) + attaches the attested
   `.crate` to a GitHub Release.

## Decisions for the maintainer
- Use a GitHub **Environment `release`** with a required reviewer (manual approval before
  any publish)? Recommended — a human gate on the irreversible publish.
- Adopt a version/CHANGELOG automation tool (e.g. `release-plz`) later, or keep the manual
  bump-tag flow? The manual flow pairs cleanly with the existing `CHANGELOG.md`.
- Mirror the same attestation approach already scaffolded in `wikiwho-data` for symmetry.

## Out of scope
Reproducible-build bit-for-bit guarantees beyond `.cargo_vcs_info.json` correspondence
(e.g. fully deterministic `.crate` across toolchains) — not required for the stated goals.
