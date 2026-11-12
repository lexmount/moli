#!/usr/bin/env python3
"""Compare response handle projection against materialize-and-scan in fresh processes."""
import argparse
import json
from pathlib import Path
import statistics
import subprocess


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--output", type=Path, default=Path("target/runtime-handle-benchmark"))
    parser.add_argument("--repeats", type=int, default=7)
    parser.add_argument("--deep-fixtures", type=Path, help="Directory captured by capture-runtime-deep-responses.mjs")
    parser.add_argument("--before-binary", type=Path, help="Pre-fix binary built with the same response_handles example")
    args = parser.parse_args()
    if args.repeats < 1:
        parser.error("--repeats must be positive")
    if args.deep_fixtures and not args.before_binary:
        parser.error("--deep-fixtures requires --before-binary")
    fixtures = args.output / "fixtures"
    fixtures.mkdir(parents=True, exist_ok=True)
    cases = []

    def case(name, result, messages, expected_handles=0):
        path = fixtures / f"{name}.json"
        with path.open("w") as output:
            json.dump({"id": 1, "result": result}, output, separators=(",", ":"))
        cases.append((name, path, messages, expected_handles))

    if not args.deep_fixtures:
        case("string-1k", {"result": {"type": "string", "value": "x" * 1024}}, 10000)
        case("object-handle", {"result": {"type": "object", "objectId": "actual"}}, 10000, 1)
        case("string-8m", {"result": {"type": "string", "value": "x" * (8 * 1024 * 1024)}}, 4)
        case("array-1m", {"result": {"type": "object", "value": list(range(1_000_000))}}, 4)
        case("by-value-100k", {"result": {"type": "object", "value": [
            {"n": i, "text": "x" * 32} for i in range(100_000)
        ]}}, 4)
        case("exception-8m", {"result": {"type": "undefined"}, "exceptionDetails": {
            "exception": {"type": "object", "objectId": "exception", "description": "x" * (8 * 1024 * 1024)}
        }}, 4, 1)
        case("properties-10k", {"result": [
            {"name": f"property{i}", "value": {"type": "object", "objectId": f"handle-{i}", "description": "Object"}, "enumerable": True}
            for i in range(10000)
        ]}, 4, 10000)

    variants = {"materialize": (args.binary, "materialize"), "project": (args.binary, "project")}
    expected_ids = {}
    manifest = None
    if args.deep_fixtures:
        manifest = json.loads((args.deep_fixtures / "manifest.json").read_text())
        cases = [(f"deep-{c['depth']}", args.deep_fixtures / c["file"], 4, len(c["expected_ids"])) for c in manifest["cases"]]
        expected_ids = {f"deep-{c['depth']}": c["expected_ids"] for c in manifest["cases"]}
    if args.before_binary:
        variants = {"before": (args.before_binary, "project"), "project": (args.binary, "project")}

    records = []
    for repeat in range(args.repeats):
        for name, fixture, messages, handles in cases:
            order = list(variants) if repeat % 2 == 0 else list(reversed(variants))
            for variant in order:
                binary, mode = variants[variant]
                output = subprocess.check_output([str(binary.resolve()), mode, str(fixture), str(messages)], text=True)
                record = json.loads(output)
                assert record["handles"] == handles * messages, record
                if name in expected_ids:
                    assert record["object_ids"] == expected_ids[name], record
                records.append({"case": name, "repeat": repeat, "variant": variant, **record})
    summary = []
    for name, _, _, _ in cases:
        row = {"case": name}
        for variant in variants:
            samples = [r for r in records if r["case"] == name and r["variant"] == variant]
            row[variant] = {key: statistics.median(r[key] for r in samples) for key in ["ns_per_message", "peak_rss_kib", "rss_kib"]}
        summary.append(row)
    report = {"scope": "Isolated Inspector ownership projection and live envelope queue; System allocator, not whole-browser RSS", "records": records, "summary": summary}
    if manifest:
        report["capture"] = manifest
    (args.output / "results.json").write_text(json.dumps(report, indent=2) + "\n")
    print(json.dumps(summary, indent=2))


if __name__ == "__main__":
    main()
