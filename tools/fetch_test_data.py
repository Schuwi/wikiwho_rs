#!/usr/bin/env python3
"""Download large test-data artifacts from the `Schuwi/wikiwho-data` release.

The representative CI subset (`dewiktionary-20240901-ci-subset.xml.zst`) is committed
into this repo via Git LFS, so contributors get it on clone and CI does not need this
script for normal runs. This script fetches the *full* 808 MB dump (and, if asked, the
subset) from the wikiwho-data release for deep/manual parity runs.

Downloads are verified against the SHA-256 sums in `tools/test-data.sha256`.

Usage:
    python3 tools/fetch_test_data.py --which full
    python3 tools/fetch_test_data.py --which all --dest dev-data/reference-dumps

Exits non-zero on any download/verification failure so callers (CI) can branch on it.
"""
from __future__ import annotations

import argparse
import hashlib
import sys
import urllib.error
import urllib.request
from pathlib import Path

DEFAULT_REPO = "Schuwi/wikiwho-data"
DEFAULT_TAG = "dewiktionary-20240901"

# logical name -> (asset filename, expected sha256)
ASSETS = {
    "full": (
        "dewiktionary-20240901-pages-meta-history.xml.zst",
        "15916141d87fd3d13c82d354100b0643d144688b6fe2ceee763ce970111d29fd",
    ),
    "ci-subset": (
        "dewiktionary-20240901-ci-subset.xml.zst",
        "04e4e335329e3d280beb790250e9f88e4d33430e4ff043c7b881d7f660135381",
    ),
}


def sha256_file(path: Path, chunk: int = 1 << 20) -> str:
    h = hashlib.sha256()
    with path.open("rb") as f:
        for block in iter(lambda: f.read(chunk), b""):
            h.update(block)
    return h.hexdigest()


def download(url: str, dest: Path) -> None:
    request = urllib.request.Request(url, headers={"User-Agent": "wikiwho-rs-ci"})
    with urllib.request.urlopen(request, timeout=120) as response:
        total = int(response.headers.get("Content-Length", 0))
        read = 0
        with dest.open("wb") as out:
            while True:
                chunk = response.read(1 << 20)
                if not chunk:
                    break
                out.write(chunk)
                read += len(chunk)
                if total:
                    print(f"\r  {read / 1e6:.1f}/{total / 1e6:.1f} MB", end="", file=sys.stderr)
        if total:
            print(file=sys.stderr)


def fetch_one(which: str, dest_dir: Path, repo: str, tag: str) -> None:
    asset, expected = ASSETS[which]
    dest = dest_dir / asset

    if dest.exists() and sha256_file(dest) == expected:
        print(f"{asset}: already present and verified")
        return

    dest_dir.mkdir(parents=True, exist_ok=True)
    url = f"https://github.com/{repo}/releases/download/{tag}/{asset}"
    print(f"downloading {url}")
    try:
        download(url, dest)
    except (urllib.error.URLError, urllib.error.HTTPError, OSError) as exc:
        dest.unlink(missing_ok=True)
        sys.exit(f"ERROR: failed to download {asset}: {exc}")

    got = sha256_file(dest)
    if got != expected:
        dest.unlink(missing_ok=True)
        sys.exit(f"ERROR: checksum mismatch for {asset}: got {got}, expected {expected}")
    print(f"{asset}: downloaded and verified ({got})")


def main() -> None:
    ap = argparse.ArgumentParser(
        description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter
    )
    ap.add_argument("--which", choices=["full", "ci-subset", "all"], default="all")
    ap.add_argument("--dest", type=Path, default=Path("dev-data/reference-dumps"))
    ap.add_argument("--repo", default=DEFAULT_REPO)
    ap.add_argument("--tag", default=DEFAULT_TAG)
    args = ap.parse_args()

    selected = ["full", "ci-subset"] if args.which == "all" else [args.which]
    for which in selected:
        fetch_one(which, args.dest, args.repo, args.tag)


if __name__ == "__main__":
    main()
