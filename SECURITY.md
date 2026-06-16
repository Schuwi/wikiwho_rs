# Security Policy

`wikiwho` is a library for parsing and analyzing Wikimedia XML dumps. It performs
no network access and requires no special privileges, so its main security
surface is robustness against malformed or hostile input (for example, a crafted
dump that makes the parser crash, hang, or allocate unbounded memory).

## Reporting a vulnerability

Please report security-relevant issues privately via GitHub's
["Report a vulnerability"](https://github.com/Schuwi/wikiwho_rs/security/advisories/new)
form rather than opening a public issue. For ordinary parsing bugs that aren't
security-sensitive, a normal [issue](https://github.com/Schuwi/wikiwho_rs/issues)
is fine.

## Verifying a release

Releases on [crates.io](https://crates.io/crates/wikiwho) are published by CI (not from a maintainer's machine), so they are independently verifiable. Each release is signed with an [SLSA build-provenance attestation](https://github.com/actions/attest-build-provenance). **Verify the artifact you actually install** — the `.crate` from crates.io (or the copy in your local `~/.cargo` cache):

```sh
ver=X.Y.Z  # replace with the version number, no leading 'v'
# fetch the exact bytes crates.io serves (crates.io requires a descriptive User-Agent)
curl -L -A "wikiwho-verify (https://github.com/Schuwi/wikiwho_rs)" \
  -o "wikiwho-$ver.crate" \
  "https://crates.io/api/v1/crates/wikiwho/$ver/download"

# verify provenance, pinned to the release workflow, the version tag, and a GitHub-hosted runner
gh attestation verify "wikiwho-$ver.crate" \
  --repo Schuwi/wikiwho_rs \
  --cert-identity "https://github.com/Schuwi/wikiwho_rs/.github/workflows/release.yml@refs/tags/v$ver" \
  --deny-self-hosted-runners
```

A successful run looks like (digest and version tag will match your download):

```text
Loaded digest sha256:24efbc63017eb2c7f0ca0086299752dfa3147d956b41ed0be726faf277b8ffd9 for file://wikiwho-0.3.3.crate
Loaded 1 attestation from GitHub API

The following policy criteria will be enforced:
- Predicate type must match:..................... https://slsa.dev/provenance/v1
- Source Repository Owner URI must match:........ https://github.com/Schuwi
- Source Repository URI must match:.............. https://github.com/Schuwi/wikiwho_rs
- Subject Alternative Name must match:........... https://github.com/Schuwi/wikiwho_rs/.github/workflows/release.yml@refs/tags/v0.3.3
- OIDC Issuer must match:........................ https://token.actions.githubusercontent.com
- Action workflow Runner Environment must match : github-hosted

✓ Verification succeeded!

The following 1 attestation matched the policy criteria

- Attestation #1
  - Build repo:..... Schuwi/wikiwho_rs
  - Build workflow:. .github/workflows/release.yml@refs/tags/v0.3.3
  - Signer repo:.... Schuwi/wikiwho_rs
  - Signer workflow: .github/workflows/release.yml@refs/tags/v0.3.3
```

`gh` fetches the attestation from GitHub by the file's content digest (crates.io does not serve it), so this **fails if the crates.io bytes were not built by this repo's release workflow** — including anything published out-of-band. The two pins check the signing certificate, the only part of an attestation a compromised build cannot forge:

- `--cert-identity` — the exact build identity: produced by `release.yml` **at the `vX.Y.Z` tag**. Because this repo uses [immutable releases](https://docs.github.com/en/code-security/concepts/supply-chain-security/immutable-releases), that tag is permanently locked to one commit, so pinning the tag also pins the source — no commit hash to look up. (`--repo` alone would accept an attestation from any workflow in the repo; this pins the workflow path *and* ref, which is also the build-signer identity.)
- `--deny-self-hosted-runners` — built on a GitHub-hosted runner, not an attacker's self-hosted one.

A pass proves the crate was produced by `release.yml` at tag `vX.Y.Z` on GitHub's infrastructure. If you want to go further: the attestation certificate also records the source commit (and the crate embeds `.cargo_vcs_info.json`), so you can open that commit on GitHub, confirm `release.yml` at it only packages and publishes, and diff the extracted crate against `git checkout v<version>`. Each immutable release additionally carries a GitHub-signed release attestation and a copy of the `.crate`, if you want a second, independent cross-link. Maintainers: see [`RELEASING.md`](RELEASING.md).
