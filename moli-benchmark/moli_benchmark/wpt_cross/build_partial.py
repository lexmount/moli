"""Synthesize a partial matrix.json + summary.json from per-engine JSON files.

Useful when the wpt_cross run is still in progress and you want a snapshot
report of the engines that already finished.
"""
from __future__ import annotations

import argparse
import json
import sys
from collections import Counter
from pathlib import Path


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("output_dir", type=Path, help="wpt_cross output directory")
    parser.add_argument("--engine", action="append", required=True,
                        help="Engine name to include (must have engine-<NAME>.json present). Repeatable.")
    args = parser.parse_args(argv)

    out_dir: Path = args.output_dir
    engine_data: dict[str, dict] = {}
    for e in args.engine:
        p = out_dir / f"engine-{e}.json"
        if not p.exists():
            print(f"error: {p} not found", file=sys.stderr)
            return 2
        engine_data[e] = json.loads(p.read_text(encoding="utf-8"))

    # Union of case_paths across engines (preserves first engine's order then appends new ones).
    seen: set[str] = set()
    ordered_cases: list[str] = []
    for e in args.engine:
        for c in engine_data[e]["cases"]:
            cp = c["case_path"]
            if cp not in seen:
                seen.add(cp)
                ordered_cases.append(cp)

    by_engine_case = {
        e: {c["case_path"]: c for c in engine_data[e]["cases"]} for e in args.engine
    }

    matrix = []
    for cp in ordered_cases:
        first_result = next(
            (
                by_engine_case[engine][cp]
                for engine in args.engine
                if cp in by_engine_case[engine]
            ),
            {},
        )
        row = {
            "case_path": cp,
            "test_type": first_result.get("test_type", "testharness"),
            "results": {},
        }
        for e in args.engine:
            r = by_engine_case[e].get(cp)
            if r is None:
                row["results"][e] = {"status": "missing"}
            else:
                row["results"][e] = {
                    "status": r["status"],
                    "duration_ms": r["duration_ms"],
                    "subtests": r["subtests"],
                    "failures": r.get("failures", []),
                    "failure_names": r.get("failure_names", []),
                    "harness_status_name": r.get("harness_status_name"),
                    "harness_message": r.get("harness_message"),
                    "error": r.get("error"),
                    "test_type": r.get("test_type", row["test_type"]),
                    "reftest_comparisons": r.get("reftest_comparisons", []),
                    "artifacts": r.get("artifacts", {}),
                }
        matrix.append(row)

    summary = {"total": len(matrix), "engines": {}, "partial": True}
    for e in args.engine:
        c = Counter()
        for row in matrix:
            c[row["results"][e]["status"]] += 1
        summary["engines"][e] = dict(c)

    matrix_path = out_dir / f"matrix.partial.{'-'.join(args.engine)}.json"
    summary_path = out_dir / f"summary.partial.{'-'.join(args.engine)}.json"
    matrix_path.write_text(json.dumps(matrix, indent=2, sort_keys=True), encoding="utf-8")
    summary_path.write_text(json.dumps(summary, indent=2, sort_keys=True), encoding="utf-8")
    print(f"wrote {matrix_path}")
    print(f"wrote {summary_path}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
