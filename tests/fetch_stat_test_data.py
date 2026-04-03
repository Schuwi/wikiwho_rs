#!/usr/bin/env python3
# SPDX-License-Identifier: MPL-2.0

from __future__ import annotations

import argparse
import csv
import hashlib
import io
import sys
import urllib.error
import urllib.request
from html.parser import HTMLParser
from pathlib import Path
from urllib.parse import urlparse


ROOT = Path(__file__).resolve().parent.parent
OUTPUT_PATH = ROOT / "tests/statistics-data/gold_standard.partial.newnames.csv"
WAYBACK_EDIT_SNAPSHOT_URL = (
    "https://web.archive.org/web/20190626204719/"
    "https://docs.google.com/spreadsheets/d/1Xvl1NXqFY_efvoZ9oj2xH86fSljLYpDNI1dt2YfISlk/"
    "edit?usp=sharing"
)

EXPECTED_SHA256 = "77e88847c1939523a57953ca54c5137e256b487660c7d88aecb70ac1327df083"
EXPECTED_HEADER = [
    "Article",
    "Number of revisions in this article",
    "Revision for whose words the authorship is determined (starting revision)",
    "Token",
]
RECOVERED_COLUMN_COUNT = 13

# Tested for 2026-01-01 dump
ARTICLE_RENAMES = {
    "Armenian Genocide": "Armenian genocide",
    "Bioglass": "Bioglass 45S5",
    "Communist Party of China": "Chinese Communist Party",
}


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description=(
            "Normalize a manually recovered partial WikiWho gold-standard CSV into the "
            "repo-local statistical test data directory."
        )
    )
    parser.add_argument(
        "--input",
        default=None,
        help=(
            "path or URL to the source input; supports CSV and HTML. "
            "If omitted, the script tries the pinned Wayback HTML snapshot."
        ),
    )
    parser.add_argument(
        "--write-recovered-csv",
        type=Path,
        default=None,
        help=(
            "optional path to write the recovered pre-rename intermediate CSV "
            "(the `gold_standard.partial.csv` equivalent)"
        ),
    )
    parser.add_argument(
        "--force",
        action="store_true",
        help="overwrite an existing output file",
    )
    return parser.parse_args()


def fetch_url(url: str) -> bytes:
    request = urllib.request.Request(
        url,
        headers={"User-Agent": "wikiwho_rs statistical test data bootstrapper"},
    )
    with urllib.request.urlopen(request, timeout=60) as response:
        return response.read()


def decode_csv(raw: bytes) -> list[list[str]]:
    text = raw.decode("utf-8-sig").replace("\r\n", "\n").replace("\r", "\n")
    rows = list(csv.reader(text.splitlines()))
    if len(rows) < 2 or rows[0][:4] != EXPECTED_HEADER:
        raise ValueError("downloaded content does not look like the expected gold-standard CSV")
    return rows


class GoogleSheetsHtmlTableParser(HTMLParser):
    def __init__(self) -> None:
        super().__init__()
        self.table_depth = 0
        self.tables: list[list[list[str]]] = []
        self.current_table: list[list[str]] | None = None
        self.current_row: list[str] | None = None
        self.current_cell: list[str] | None = None

    def handle_starttag(self, tag: str, attrs: list[tuple[str, str | None]]) -> None:
        if tag == "table":
            self.table_depth += 1
            if self.table_depth == 1:
                self.current_table = []
        elif self.table_depth == 1 and tag == "tr":
            self.current_row = []
        elif self.current_row is not None and tag in ("td", "th"):
            self.current_cell = []
        elif self.current_cell is not None and tag == "br":
            self.current_cell.append(" ")

    def handle_endtag(self, tag: str) -> None:
        if tag in ("td", "th") and self.current_cell is not None:
            self.current_row.append("".join(self.current_cell).strip())
            self.current_cell = None
        elif tag == "tr" and self.current_row is not None:
            self.current_table.append(self.current_row)
            self.current_row = None
        elif tag == "table":
            if self.table_depth == 1 and self.current_table is not None:
                self.tables.append(self.current_table)
                self.current_table = None
            self.table_depth -= 1

    def handle_data(self, data: str) -> None:
        if self.current_cell is not None:
            self.current_cell.append(data)


def recover_partial_csv_rows_from_html(raw: bytes) -> list[list[str]]:
    html_text = raw.decode("utf-8", errors="replace")
    parser = GoogleSheetsHtmlTableParser()
    parser.feed(html_text)

    for table in parser.tables:
        for header_index, row in enumerate(table):
            candidate = row[1 : 1 + RECOVERED_COLUMN_COUNT]
            if candidate[:4] == EXPECTED_HEADER:
                recovered_rows: list[list[str]] = []
                for raw_row in table[header_index:]:
                    cells = raw_row[1 : 1 + RECOVERED_COLUMN_COUNT]
                    if not any(cells):
                        continue
                    cells = cells + [""] * (RECOVERED_COLUMN_COUNT - len(cells))
                    recovered_rows.append(cells)
                if recovered_rows and recovered_rows[0][:4] == EXPECTED_HEADER:
                    return recovered_rows

    raise ValueError("could not find the expected spreadsheet table in the HTML snapshot")


def normalize_rows(rows: list[list[str]]) -> list[list[str]]:
    normalized = []
    for row in rows:
        row = list(row)
        if row and row[0] in ARTICLE_RENAMES:
            row[0] = ARTICLE_RENAMES[row[0]]
        normalized.append(row)
    return normalized


def render_csv(rows: list[list[str]]) -> bytes:
    buffer = io.StringIO()
    writer = csv.writer(buffer, lineterminator="\n")
    writer.writerows(rows)
    return buffer.getvalue().encode("utf-8")


def sha256_hex(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def write_output(data: bytes) -> None:
    OUTPUT_PATH.parent.mkdir(parents=True, exist_ok=True)
    OUTPUT_PATH.write_bytes(data)


def is_url(value: str) -> bool:
    parsed = urlparse(value)
    return parsed.scheme in ("http", "https")


def load_rows_from_source(source: str) -> tuple[list[list[str]], str]:
    if is_url(source):
        raw = fetch_url(source)
        if source.endswith(".csv") or "format=csv" in source or "output=csv" in source:
            return decode_csv(raw), source
        return recover_partial_csv_rows_from_html(raw), source

    path = Path(source)
    raw = path.read_bytes()
    if path.suffix.lower() == ".csv":
        return decode_csv(raw), str(path)
    return recover_partial_csv_rows_from_html(raw), str(path)


def main() -> int:
    args = parse_args()

    if OUTPUT_PATH.exists() and not args.force:
        print(f"{OUTPUT_PATH} already exists; pass --force to overwrite", file=sys.stderr)
        return 0

    attempted_sources: list[str] = []
    rows: list[list[str]] | None = None
    source: str | None = None

    candidate_sources: list[str] = []
    if args.input is not None:
        candidate_sources.append(str(args.input))
    else:
        candidate_sources.append(WAYBACK_EDIT_SNAPSHOT_URL)

    for candidate in candidate_sources:
        attempted_sources.append(candidate)
        try:
            rows, source = load_rows_from_source(candidate)
            break
        except (FileNotFoundError, urllib.error.URLError, urllib.error.HTTPError, ValueError) as exc:
            print(f"Skipping {candidate}: {exc}", file=sys.stderr)

    if rows is None or source is None:
        print(
            "Could not obtain the partial gold-standard data from any input source.\n"
            "Tried:\n  - " + "\n  - ".join(attempted_sources),
            file=sys.stderr,
        )
        return 1

    if args.write_recovered_csv is not None:
        recovered_path = args.write_recovered_csv
        if not recovered_path.is_absolute():
            recovered_path = ROOT / recovered_path
        recovered_path.parent.mkdir(parents=True, exist_ok=True)
        recovered_path.write_bytes(render_csv(rows))

    normalized_bytes = render_csv(normalize_rows(rows))
    digest = sha256_hex(normalized_bytes)
    if digest != EXPECTED_SHA256:
        print(
            "Normalized gold-standard CSV checksum mismatch.\n"
            f"Expected: {EXPECTED_SHA256}\n"
            f"Actual:   {digest}\n"
            f"Input:    {source}",
            file=sys.stderr,
        )
        return 1

    write_output(normalized_bytes)
    print(f"Wrote {OUTPUT_PATH.relative_to(ROOT)}")
    if args.write_recovered_csv is not None:
        print(f"Recovered CSV: {recovered_path}")
    print(f"Input: {source}")
    print(f"Snapshot provenance: {WAYBACK_EDIT_SNAPSHOT_URL}")
    print(f"SHA-256: {digest}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
