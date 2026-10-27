#!/usr/bin/env python3
"""Measure the two-window frontier across complementary solver policies.

Correctness control: every generated case is known to be NO.  A definite YES,
non-zero exit, crash, or malformed first line is recorded as a failure rather
than folded into timing statistics.  UNKNOWN is a resource-limit outcome.
"""

from __future__ import annotations

import argparse
import csv
import statistics
import subprocess
import time
from pathlib import Path

SIZES = (20, 24, 32, 48, 64, 96, 128, 256, 512)
CONFIGS = {
    "automata_auto": ("--backend", "automata", "--orientation", "auto"),
    "hkc_forward": ("--backend", "hkc", "--orientation", "forward"),
    "portfolio_forward": ("--backend", "portfolio", "--orientation", "forward"),
    "portfolio_auto": ("--backend", "portfolio", "--orientation", "auto"),
}
QUERIES = ("includes", "equivalent")


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser()
    parser.add_argument("--binary", default="target/release/regexrel")
    parser.add_argument("--max-states", type=int, default=60_000_000)
    parser.add_argument("--timeout-ms", type=int, default=1_000)
    parser.add_argument("--repeats", type=int, default=3)
    parser.add_argument("--output", default="bench/portfolio_frontier.csv")
    return parser.parse_args()


def patterns(n: int) -> tuple[str, str]:
    left = f"[RW]*R[RW]{{{n}}}#[RW]{{{n}}}R[RW]*"
    right = f"[RW]*W[RW]{{{n}}}#[RW]{{{n}}}R[RW]*"
    return left, right


def main() -> int:
    args = parse_args()
    binary = Path(args.binary)
    if not binary.is_file():
        raise SystemExit(f"binary not found: {binary}; run cargo build --release first")
    if args.repeats < 1:
        raise SystemExit("--repeats must be >= 1")

    rows: list[dict[str, object]] = []
    hard_timeout = max(2.0, args.timeout_ms / 1000.0 + 2.0)
    for config_name, config_args in CONFIGS.items():
        for n in SIZES:
            left, right = patterns(n)
            for query in QUERIES:
                for repeat in range(1, args.repeats + 1):
                    cmd = [
                        str(binary),
                        "--max-states",
                        str(args.max_states),
                        "--timeout-ms",
                        str(args.timeout_ms),
                        *config_args,
                        query,
                        left,
                        right,
                    ]
                    started = time.perf_counter()
                    try:
                        proc = subprocess.run(
                            cmd,
                            text=True,
                            stdout=subprocess.PIPE,
                            stderr=subprocess.PIPE,
                            timeout=hard_timeout,
                            check=False,
                        )
                        wall_ms = (time.perf_counter() - started) * 1000.0
                        first = proc.stdout.splitlines()[0].strip() if proc.stdout else ""
                        if proc.returncode != 0:
                            outcome = "ERROR"
                        elif first == "NO":
                            outcome = "SOLVED"
                        elif first == "UNKNOWN":
                            outcome = "LIMIT"
                        elif first == "YES":
                            outcome = "WRONG"
                        else:
                            outcome = "MALFORMED"
                        rows.append(
                            {
                                "config": config_name,
                                "n": n,
                                "query": query,
                                "repeat": repeat,
                                "outcome": outcome,
                                "wall_ms": f"{wall_ms:.3f}",
                                "exit_code": proc.returncode,
                                "stdout_first_line": first,
                                "stderr": proc.stderr.strip().replace("\n", "\\n"),
                            }
                        )
                    except subprocess.TimeoutExpired as exc:
                        wall_ms = (time.perf_counter() - started) * 1000.0
                        rows.append(
                            {
                                "config": config_name,
                                "n": n,
                                "query": query,
                                "repeat": repeat,
                                "outcome": "HARNESS_TIMEOUT",
                                "wall_ms": f"{wall_ms:.3f}",
                                "exit_code": "",
                                "stdout_first_line": "",
                                "stderr": str(exc).replace("\n", "\\n"),
                            }
                        )

    output = Path(args.output)
    output.parent.mkdir(parents=True, exist_ok=True)
    fields = [
        "config",
        "n",
        "query",
        "repeat",
        "outcome",
        "wall_ms",
        "exit_code",
        "stdout_first_line",
        "stderr",
    ]
    with output.open("w", newline="", encoding="utf-8") as handle:
        writer = csv.DictWriter(handle, fieldnames=fields)
        writer.writeheader()
        writer.writerows(rows)

    print(f"wrote {output} ({len(rows)} runs)")
    for config_name in CONFIGS:
        config_rows = [row for row in rows if row["config"] == config_name]
        solved = sum(row["outcome"] == "SOLVED" for row in config_rows)
        bad = sum(row["outcome"] in {"WRONG", "ERROR", "MALFORMED", "HARNESS_TIMEOUT"} for row in config_rows)
        solved_times = [
            float(row["wall_ms"])
            for row in config_rows
            if row["outcome"] == "SOLVED"
        ]
        median = statistics.median(solved_times) if solved_times else float("nan")
        print(
            f"{config_name:18s} solved={solved:3d}/{len(config_rows):3d} "
            f"bad={bad:2d} solved_median_ms={median:.3f}"
        )

    return 1 if any(row["outcome"] in {"WRONG", "ERROR", "MALFORMED", "HARNESS_TIMEOUT"} for row in rows) else 0


if __name__ == "__main__":
    raise SystemExit(main())
