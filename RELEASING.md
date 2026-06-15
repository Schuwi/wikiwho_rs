# Releasing `wikiwho`

Releases are published to crates.io by CI (`.github/workflows/release.yml`), triggered by a
`v*` tag. There is no `CARGO_REGISTRY_TOKEN` anywhere — publishing uses crates.io
[Trusted Publishing](https://crates.io/docs/trusted-publishing) (OIDC) — and a manual approval
on the `release` GitHub Environment is required before anything is published.

## Versioning (continuous bump)

The `version` in `Cargo.toml` is kept as the *next* version to publish and is bumped in the PR
that makes the change require it — not at release time. `ci.yml`'s `semver` job enforces this:
`cargo-semver-checks` compares the public API against the latest crates.io release and fails the
PR if the version doesn't reflect the change (`0.x` rules: a breaking change needs a minor bump,
e.g. `0.3.x` → `0.4.0`). So by the time you release, `main` already carries a correct version.

## Procedure

1. Ensure `CHANGELOG.md` is up to date for the release, and `version` in `Cargo.toml` is the
   version to publish (it was bumped in whichever PR introduced the breaking change).
2. Merge to `main` and wait for CI (`ci.yml`) to go green.
3. Tag that commit and push the tag:
   ```sh
   git tag vX.Y.Z && git push origin vX.Y.Z
   ```
4. Approve the pending `release` deployment on the Actions run. CI then runs the gates and
   publishes.

> **Always tag the tip of a green `main`.** The release refuses to publish unless `ci.yml`
> succeeded for the *exact* tagged commit, and `ci.yml` only runs for the tip commit of a push
> to `main`. Tagging an intermediate or off-`main` commit will be (correctly) blocked.

## Gates (all must pass before publish)

1. **Green CI** — the `ci.yml` run for the tagged commit must have concluded `success`. No checks
   are re-run; the step polls up to 15 min in case the tag was pushed before CI finished. This is
   what enforces SemVer at release time: ci.yml's `semver` job already checked the version against
   the crates.io baseline for that commit (over `--all-features`, including `python-diff`).
2. **Version == tag** — `Cargo.toml` version must equal the tag.

`cargo publish` also performs a verification build of the packaged crate, so the published bytes
are compile-verified even though the test suite is not re-run.

## What a release produces

Order of operations: run the gates, `cargo publish`, create the immutable GitHub release, then
**attest last**. `cargo publish` leaves the exact uploaded tarball in `target/package/`; that one
file is what gets attached to the release and attested, so the attestation subject, the release
asset, and the crates.io bytes are identical **by construction** — no assumption that cargo
packaging is byte-deterministic across invocations.

**Attestation is deliberately the final step.** A valid build-provenance attestation for a tag can
therefore exist only once the crate is actually on crates.io *and* the immutable release is
published — there is never an orphan attestation binding a `.crate` to a tag that never shipped.
The pipeline fails closed: if any step fails, at worst you get a published-but-unattested release
(the consumer's verify fails safe), never an attestation for an artifact that isn't on crates.io.

- **The crate on crates.io** via Trusted Publishing — the one canonical, consumed artifact. It
  embeds `.cargo_vcs_info.json` for source correspondence.
- **An immutable GitHub Release.** Publishing it locks the tag to the commit — which is what makes
  consumer verification *by tag name* sound — and auto-generates a GitHub-signed release
  attestation over the asset digest. The `.crate` is attached as an independent cross-link,
  deliberately *not* the documented verification target.
- **An SLSA build-provenance attestation** (Sigstore/Rekor), bound to the `.crate`'s content digest
  and carrying the build identity (`release.yml@refs/tags/vX.Y.Z`, the runner environment, the
  source commit). Retrieved by `gh attestation verify` from GitHub, not from crates.io.

Publishing to crates.io *before* creating the immutable release means a publish failure strands
nothing (no release is created, the version name stays reusable); the trade-off is that the tag is
locked a moment after the crate appears on crates.io rather than before. OIDC auth is obtained just
before `cargo publish` (the first irreversible step), so an auth failure is fully recoverable.

Consumers verify the artifact they actually install — the crates.io download or their `~/.cargo`
cache copy. See the README's "Verifying a release" section.

> **First-release check.** On the first tagged release, run the README's verify command against the
> published crate to confirm the whole chain works end to end (OIDC publish, `--cert-identity`
> match, immutable release).

## Security model and limitations

Consumer verification (see the README's "Verifying a release") enforces everything against the
attestation's **signing certificate** — via `--cert-identity` (the build identity: the `release.yml`
workflow at the `vX.Y.Z` tag, which is also the build-signer identity) and `--deny-self-hosted-runners`
(the runner environment). These certificate fields cannot be forged by a workflow even with its OIDC
token; the SLSA `predicate` *can* be, so it is not relied on.

Pinning the **tag** (rather than the commit) is sound because of two immutability backstops: the
repo has **immutable releases** enabled, so a published tag is permanently locked to its commit and
cannot be moved to a malicious commit and re-attested under the same `…@refs/tags/vX.Y.Z` identity;
and a crates.io version can never be replaced once published. Because the workflow is **non-reusable
and triggered only by a tag push**, the signer equals the source — so the single `--cert-identity`
pin covers the build script, and there is no signer != source gap to check separately.

What verification does and does not protect against:

- **Wrong repo / wrong workflow / self-hosted runner → caught.** An artifact attested by a
  different repo, a different workflow, or on a self-hosted runner fails the pinned verify.
- **A hijacked `release.yml` → *not* caught by verification.** If an attacker runs code inside the
  release job (a compromised build dependency, a tampered action, or an unreviewed change to
  `release.yml`), the artifact genuinely carries this repo + workflow identity and passes every
  flag. The defenses are upstream:
  - the `release` environment's required-reviewer gate (no silent release);
  - branch protection + CODEOWNERS on `main` and `.github/` so `release.yml` can't change unreviewed;
  - third-party actions pinned to full commit SHAs in `release.yml` (Dependabot keeps them
    current); the other workflows still use tags, which is lower-risk as they hold no publish or
    signing credentials;
  - least-privilege tokens and monitoring the public attestation/Rekor log.

  It is also why a consumer's final step is to read `release.yml` *at the pinned commit*.

Attestations are **not revocable.** Sigstore uses short-lived certificates and an append-only
transparency log (Rekor), so there is no key or certificate to revoke and the log entry is
permanent. Deleting an attestation from GitHub's store does not invalidate it (Rekor still has it;
offline bundles still verify). The response to a bad-but-validly-signed release is to **yank** it
on crates.io, file a RustSec advisory, and publish a fixed version — the transparency log aids
detection and forensics, not revocation.

## One-time setup (done)

- crates.io Trusted Publishing: repo `Schuwi/wikiwho_rs`, workflow `release.yml`, environment
  `release`.
- GitHub Environment `release` with a required reviewer.
- **Immutable releases** enabled (repo/org setting). Verification by tag name depends on this; if
  it is ever disabled, fall back to pinning the commit (`--source-digest`, read from the
  attestation certificate or the crate's `.cargo_vcs_info.json`).

## Possible future additions

- [`release-plz`](https://release-plz.dev) for automated version-bump + `CHANGELOG.md` "release
  PRs" — most useful for workspaces or frequent releases; the current single-crate, hand-curated
  flow is simpler and was kept deliberately.
