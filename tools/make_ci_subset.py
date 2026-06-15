#!/usr/bin/env python3
"""Build a small, representative dump subset for CI real-page parity tests.

The full Wikimedia dump (~808 MB) is too large to download on every CI run, so the
deterministic real-page parity job (`random_pages_100`, `known_bad_example_familia`,
`known_bad_example_hallo`) runs against this distilled subset on pull requests, and
against the full dump on pushes to `main`.

The subset is built by *raw XML extraction* (whole `<page>…</page>` blocks copied
byte-for-byte), so the pages are identical to those in the full dump — parity results
on the subset match what the full dump would produce.

Selection:
  * always include the named regression pages (`familia`, `Hallo` at ns 0),
  * fill with main-namespace (ns 0) pages whose raw block is <= MAX_BLOCK_BYTES
    (skips multi-megabyte histories to keep the subset small and CI fast),
  * stop once TARGET_PAGES filler pages are collected and all required pages found.

Output is a self-contained `<mediawiki>` document (preamble + selected pages + footer),
zstd-compressed, plus a `.sha256` sidecar. Upload the `.xml.zst` to the
`Schuwi/wikiwho-data` release; `tools/fetch_test_data.py` downloads + verifies it.

Usage:
    python3 tools/make_ci_subset.py \
        [--src dev-data/reference-dumps/dewiktionary-20240901-pages-meta-history.xml.zst] \
        [--out dev-data/reference-dumps/dewiktionary-20240901-ci-subset.xml.zst]
"""
from __future__ import annotations

import argparse
import hashlib
import io
import re
import sys
from pathlib import Path

import zstandard as zstd

TARGET_PAGES = 500
MAX_BLOCK_BYTES = 64 * 1024
# (namespace, title) pairs that MUST be present (regardless of block size)
REQUIRED = {(0, b"familia"), (0, b"Hallo")}

TITLE_RE = re.compile(rb"<title>(.*?)</title>", re.S)
NS_RE = re.compile(rb"<ns>(-?\d+)</ns>")

DEFAULT_SRC = "dev-data/reference-dumps/dewiktionary-20240901-pages-meta-history.xml.zst"
DEFAULT_OUT = "dev-data/reference-dumps/dewiktionary-20240901-ci-subset.xml.zst"


def build(src: Path, out: Path) -> None:
    preamble = bytearray()
    selected: list[bytes] = []
    found_required: set[tuple[int, bytes]] = set()
    filler = 0

    dctx = zstd.ZstdDecompressor()
    with src.open("rb") as fh, dctx.stream_reader(fh) as raw:
        stream = io.BufferedReader(raw, buffer_size=1 << 20)
        page_lines: list[bytes] | None = None

        for line in iter(stream.readline, b""):
            if page_lines is None:
                if line.lstrip().startswith(b"<page>"):
                    page_lines = [line]
                else:
                    preamble += line
                continue

            page_lines.append(line)
            if not line.lstrip().startswith(b"</page>"):
                continue

            block = b"".join(page_lines)
            page_lines = None

            m_title = TITLE_RE.search(block)
            m_ns = NS_RE.search(block)
            title = m_title.group(1) if m_title else b""
            ns = int(m_ns.group(1)) if m_ns else None
            key = (ns, title)

            if key in REQUIRED:
                selected.append(block)
                found_required.add(key)
            elif filler < TARGET_PAGES and ns == 0 and len(block) <= MAX_BLOCK_BYTES:
                selected.append(block)
                filler += 1

            if filler >= TARGET_PAGES and REQUIRED <= found_required:
                break

    missing = REQUIRED - found_required
    if missing:
        names = ", ".join(f"ns{ns}:{title.decode(errors='replace')}" for ns, title in missing)
        sys.exit(f"ERROR: required pages not found in dump: {names}")

    # Keep a stable order: required pages first, then filler (already in dump order).
    document = bytearray(preamble)
    for block in selected:
        document += block
    document += b"</mediawiki>\n"

    cctx = zstd.ZstdCompressor(level=19)
    compressed = cctx.compress(bytes(document))
    out.parent.mkdir(parents=True, exist_ok=True)
    out.write_bytes(compressed)

    digest = hashlib.sha256(compressed).hexdigest()
    sha_path = out.with_suffix(out.suffix + ".sha256")
    sha_path.write_text(f"{digest}  {out.name}\n")

    print(f"wrote {out} ({len(compressed):,} bytes compressed, "
          f"{len(document):,} bytes raw)")
    print(f"pages: {len(selected)} ({filler} filler + {len(found_required)} required)")
    print(f"sha256: {digest}")
    print(f"sidecar: {sha_path}")


def main() -> None:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--src", type=Path, default=Path(DEFAULT_SRC))
    ap.add_argument("--out", type=Path, default=Path(DEFAULT_OUT))
    args = ap.parse_args()

    if not args.src.exists():
        sys.exit(f"ERROR: source dump not found: {args.src}")
    build(args.src, args.out)


if __name__ == "__main__":
    main()
