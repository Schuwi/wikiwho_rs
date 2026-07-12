#!/usr/bin/env python3
"""Benchmark Rust WikiWho against the original Python implementation on real pages."""

from __future__ import annotations

import argparse
import json
import statistics
import subprocess
import sys
import tempfile
import time
from pathlib import Path
from typing import Any, Sequence


ROOT = Path(__file__).resolve().parents[1]
DEFAULT_DUMP = ROOT / "dev-data/reference-dumps/dewiktionary-20240901-ci-subset.xml.zst"
DEFAULT_BINARY = ROOT / "target/release/wikiwho-benchmark"


class Deleted:
    def __init__(self, text: bool = False, restricted: bool = False) -> None:
        self.text = text
        self.restricted = restricted


class Timestamp:
    def __init__(self, value: str) -> None:
        self.value = value

    def long_format(self) -> str:
        return self.value


class User:
    def __init__(self, user_id: int | None, text: str) -> None:
        self.id = user_id
        self.text = text


class Revision:
    def __init__(self, raw: dict[str, Any]) -> None:
        self.id = raw["id"]
        text = raw["text"]
        if isinstance(text, dict) and "Normal" in text:
            self.text = text["Normal"]
            self.deleted = Deleted()
        else:
            self.text = None
            self.deleted = Deleted(text=True)
        self.timestamp = Timestamp(raw["timestamp"])
        contributor = raw["contributor"]
        self.user = User(contributor.get("id"), contributor.get("username", ""))
        self.comment = raw.get("comment")
        self.minor = raw.get("minor", False)
        sha1 = raw.get("sha1")
        if isinstance(sha1, list):
            self.sha1 = bytes(sha1).decode("ascii")
        else:
            self.sha1 = sha1


def _python_worker(corpus: Path) -> int:
    try:
        from WikiWho.wikiwho import Wikiwho
    except ModuleNotFoundError as exc:
        raise SystemExit(
            "The original Python WikiWho is not installed for this interpreter. "
            "Activate the project virtualenv or pass --python /path/to/venv/bin/python."
        ) from exc

    pages = revisions = text_bytes = analysed_revisions = tokens = checksum = 0
    algorithm_seconds = 0.0
    with corpus.open(encoding="utf-8") as lines:
        for line_number, line in enumerate(lines, 1):
            try:
                raw = json.loads(line)
            except json.JSONDecodeError as exc:
                raise SystemExit(f"invalid corpus line {line_number}: {exc}") from exc
            page_revisions = [Revision(revision) for revision in raw["revisions"]]
            pages += 1
            revisions += len(page_revisions)
            text_bytes += sum(len((revision.text or "").encode("utf-8")) for revision in page_revisions)

            wikiwho = Wikiwho(raw["title"])
            start = time.perf_counter()
            wikiwho.analyse_article_from_xml_dump(page_revisions)
            algorithm_seconds += time.perf_counter() - start

            analysed_revisions += len(wikiwho.ordered_revisions)
            tokens += len(wikiwho.tokens)
            checksum = (
                checksum * 1_099_511_628_211
                + len(wikiwho.tokens)
                + (len(wikiwho.ordered_revisions) << 32)
            ) & ((1 << 64) - 1)

    print(
        json.dumps(
            {
                "implementation": "python",
                "mode": "algorithm",
                "pages": pages,
                "revisions": revisions,
                "text_bytes": text_bytes,
                "analysed_revisions": analysed_revisions,
                "tokens": tokens,
                "seconds": algorithm_seconds,
                "checksum": checksum,
            },
            separators=(",", ":"),
        )
    )
    return 0


def _python_xml_worker(mode: str, limit: int, namespaces: set[int], inputs: list[Path]) -> int:
    try:
        from mwxml import Dump
        from WikiWho.wikiwho import Wikiwho
    except ModuleNotFoundError as exc:
        raise SystemExit(
            "The original Python WikiWho and its mwxml dependency are not installed for this "
            "interpreter. Activate the project virtualenv or pass --python."
        ) from exc

    pages = revisions = text_bytes = analysed_revisions = tokens = checksum = 0
    start = time.perf_counter()
    stop = False
    for input_path in inputs:
        with input_path.open("rb") as stream:
            dump = Dump.from_file(stream)
            for page in dump:
                if namespaces and page.namespace not in namespaces:
                    continue
                page_revisions = list(page)
                pages += 1
                revisions += len(page_revisions)
                text_bytes += sum(len((revision.text or "").encode("utf-8")) for revision in page_revisions)
                if mode == "end-to-end":
                    wikiwho = Wikiwho(page.title)
                    wikiwho.analyse_article_from_xml_dump(page_revisions)
                    analysed_revisions += len(wikiwho.ordered_revisions)
                    tokens += len(wikiwho.tokens)
                    checksum = (
                        checksum * 1_099_511_628_211
                        + len(wikiwho.tokens)
                        + (len(wikiwho.ordered_revisions) << 32)
                    ) & ((1 << 64) - 1)
                else:
                    checksum = (
                        checksum * 1_099_511_628_211 + len(page_revisions) + len(page.title)
                    ) & ((1 << 64) - 1)
                if pages >= limit:
                    stop = True
                    break
        if stop:
            break
    seconds = time.perf_counter() - start
    if pages == 0:
        raise SystemExit("no pages matched the requested inputs and filters")
    print(
        json.dumps(
            {
                "implementation": "python",
                "mode": mode,
                "pages": pages,
                "revisions": revisions,
                "text_bytes": text_bytes,
                "analysed_revisions": analysed_revisions,
                "tokens": tokens,
                "seconds": seconds,
                "checksum": checksum,
            },
            separators=(",", ":"),
        )
    )
    return 0


def run_json(command: Sequence[str]) -> dict[str, Any]:
    completed = subprocess.run(command, check=False, text=True, capture_output=True)
    if completed.returncode:
        detail = completed.stderr.strip() or completed.stdout.strip()
        raise SystemExit(f"command failed ({completed.returncode}): {' '.join(command)}\n{detail}")
    try:
        return json.loads(completed.stdout)
    except json.JSONDecodeError as exc:
        raise SystemExit(
            f"command returned invalid JSON: {' '.join(command)}\n{completed.stdout.strip()}"
        ) from exc


def build_rust(binary: Path) -> None:
    print("Building optimized Rust benchmark worker...", file=sys.stderr)
    completed = subprocess.run(
        ["cargo", "build", "--release", "--features", "cli", "--bin", "wikiwho-benchmark"],
        cwd=ROOT,
        check=False,
    )
    if completed.returncode:
        raise SystemExit(completed.returncode)
    if not binary.is_file():
        raise SystemExit(f"Rust benchmark binary was not created at {binary}")


def validate_report(report: dict[str, Any], expected: dict[str, Any]) -> None:
    for key in ("pages", "revisions", "text_bytes"):
        if report[key] != expected[key]:
            raise SystemExit(
                f"{report['implementation']} worker saw {report[key]} {key}; expected {expected[key]}"
            )


def describe(values: list[float]) -> dict[str, float]:
    return {
        "median": statistics.median(values),
        "min": min(values),
        "max": max(values),
    }


def print_report(result: dict[str, Any]) -> None:
    summary = result["corpus"]
    print(
        f"Corpus: {summary['pages']:,} pages, {summary['revisions']:,} revisions, "
        f"{summary['text_bytes'] / (1024 * 1024):.2f} MiB revision text"
    )
    print(f"Repetitions: {result['repetitions']} (median)\n")
    print("Mode             Rust median  Python median     Speedup")
    print("---------------  -----------  -------------  ----------")
    for mode, values in result["modes"].items():
        print(
            f"{mode:<15}  {values['rust']['median']:>10.3f}s  "
            f"{values['python']['median']:>12.3f}s  {values['speedup']:>9.2f}x"
        )


def benchmark(args: argparse.Namespace) -> int:
    binary = args.rust_binary.resolve()
    if not args.no_build:
        build_rust(binary)
    elif not binary.is_file():
        raise SystemExit(f"Rust benchmark binary does not exist: {binary}")

    with tempfile.TemporaryDirectory(prefix="wikiwho-bench-") as temporary:
        if args.corpus:
            corpus = args.corpus.resolve()
            if not corpus.is_file():
                raise SystemExit(f"corpus does not exist: {corpus}")
            corpus_summary = None
        else:
            xml_inputs = []
            print("Materializing uncompressed XML outside the timed region...", file=sys.stderr)
            for index, input_path in enumerate(args.inputs):
                xml_path = Path(temporary) / f"input-{index}.xml"
                completed = subprocess.run(
                    [str(binary), "decompress", "--output", str(xml_path), str(input_path.resolve())],
                    check=False,
                )
                if completed.returncode:
                    raise SystemExit(completed.returncode)
                xml_inputs.append(xml_path)
            corpus = Path(temporary) / "corpus.jsonl"
            command = [str(binary), "prepare", "--output", str(corpus), "--limit", str(args.pages)]
            for namespace in args.namespace:
                command.extend(("--namespace", str(namespace)))
            command.extend(str(path) for path in xml_inputs)
            print("Preparing the shared real-page corpus...", file=sys.stderr)
            corpus_summary = run_json(command)

        if args.corpus:
            xml_inputs = []
            selected_modes = ["algorithm"]
        else:
            selected_modes = args.mode or ["parse", "algorithm", "end-to-end"]

        commands: dict[str, dict[str, list[str]]] = {}
        namespace_args = [item for namespace in args.namespace for item in ("--namespace", str(namespace))]
        for mode in selected_modes:
            if mode == "algorithm":
                commands[mode] = {
                    "rust": [str(binary), "run", "--corpus", str(corpus)],
                    "python": [str(args.python), str(Path(__file__).resolve()), "_python-worker", str(corpus)],
                }
            else:
                commands[mode] = {
                    "rust": [
                        str(binary), "run-xml", "--mode", mode, "--limit", str(args.pages),
                        *namespace_args, *(str(path) for path in xml_inputs),
                    ],
                    "python": [
                        str(args.python), str(Path(__file__).resolve()), "_python-xml-worker", mode,
                        str(args.pages), json.dumps(args.namespace), *(str(path) for path in xml_inputs),
                    ],
                }

        print("Warming up implementations...", file=sys.stderr)
        for _ in range(args.warmups):
            for mode in selected_modes:
                for name in ("rust", "python"):
                    run_json(commands[mode][name])

        reports = {
            mode: {"rust": [], "python": []} for mode in selected_modes
        }
        print("Running measured repetitions...", file=sys.stderr)
        for repetition in range(args.repetitions):
            order = ("rust", "python") if repetition % 2 == 0 else ("python", "rust")
            for mode in selected_modes:
                for name in order:
                    reports[mode][name].append(run_json(commands[mode][name]))

        expected = corpus_summary or reports["algorithm"]["rust"][0]
        for mode_reports in reports.values():
            for implementation_reports in mode_reports.values():
                for report in implementation_reports:
                    validate_report(report, expected)

        mode_results = {}
        for mode, mode_reports in reports.items():
            timings = {
                name: describe([report["seconds"] for report in implementation_reports])
                for name, implementation_reports in mode_reports.items()
            }
            if timings["rust"]["median"] == 0:
                raise SystemExit(f"Rust {mode} timing resolution was too small; benchmark more pages")
            mode_results[mode] = {
                **timings,
                "speedup": timings["python"]["median"] / timings["rust"]["median"],
                "last_reports": {name: values[-1] for name, values in mode_reports.items()},
            }
        result = {
            "corpus": {key: expected[key] for key in ("pages", "revisions", "text_bytes")},
            "repetitions": args.repetitions,
            "warmups": args.warmups,
            "modes": mode_results,
        }
        print_report(result)
        if args.json_output:
            args.json_output.write_text(json.dumps(result, indent=2) + "\n", encoding="utf-8")
            print(f"Machine-readable report: {args.json_output}")
    return 0


def positive_int(value: str) -> int:
    parsed = int(value)
    if parsed < 1:
        raise argparse.ArgumentTypeError("must be at least 1")
    return parsed


def nonnegative_int(value: str) -> int:
    parsed = int(value)
    if parsed < 0:
        raise argparse.ArgumentTypeError("must be at least 0")
    return parsed


def parse_args(argv: Sequence[str]) -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Compare Rust and original Python WikiWho on real revision histories."
    )
    parser.add_argument(
        "inputs",
        nargs="*",
        type=Path,
        default=[DEFAULT_DUMP],
        help="MediaWiki history dumps (.xml, .bz2, .gz, .zst); defaults to the committed CI subset",
    )
    parser.add_argument("--pages", type=positive_int, default=10, help="maximum pages to select (default: 10)")
    parser.add_argument(
        "--namespace", "-n", action="append", type=int, default=[], help="only select this namespace (repeatable)"
    )
    parser.add_argument("--repetitions", "-r", type=positive_int, default=3, help="measured runs (default: 3)")
    parser.add_argument("--warmups", type=nonnegative_int, default=1, help="unmeasured runs (default: 1)")
    parser.add_argument(
        "--mode", action="append", choices=("parse", "algorithm", "end-to-end"),
        help="benchmark only this mode (repeatable; default: all three)",
    )
    parser.add_argument("--python", type=Path, default=Path(sys.executable), help="Python interpreter with WikiWho installed")
    parser.add_argument("--rust-binary", type=Path, default=DEFAULT_BINARY, help=argparse.SUPPRESS)
    parser.add_argument("--no-build", action="store_true", help="reuse an existing release benchmark worker")
    parser.add_argument("--corpus", type=Path, help="reuse a prepared JSONL corpus instead of parsing inputs")
    parser.add_argument("--json-output", type=Path, help="also write the full result as JSON")
    args = parser.parse_args(argv)
    if args.mode:
        args.mode = list(dict.fromkeys(args.mode))
    if args.corpus and args.mode and args.mode != ["algorithm"]:
        parser.error("--corpus can only be used with --mode algorithm")
    if args.corpus and args.inputs != [DEFAULT_DUMP]:
        parser.error("INPUT and --corpus cannot be used together")
    for path in args.inputs if not args.corpus else []:
        if not path.is_file():
            parser.error(f"input does not exist: {path}")
    return args


def main(argv: Sequence[str] | None = None) -> int:
    actual = list(sys.argv[1:] if argv is None else argv)
    if actual[:1] == ["_python-worker"]:
        if len(actual) != 2:
            raise SystemExit("_python-worker requires CORPUS")
        return _python_worker(Path(actual[1]))
    if actual[:1] == ["_python-xml-worker"]:
        if len(actual) < 5:
            raise SystemExit("_python-xml-worker requires MODE LIMIT NAMESPACES INPUT...")
        return _python_xml_worker(
            actual[1], int(actual[2]), set(json.loads(actual[3])), [Path(path) for path in actual[4:]]
        )
    return benchmark(parse_args(actual))


if __name__ == "__main__":
    raise SystemExit(main())
