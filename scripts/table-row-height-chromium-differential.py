#!/usr/bin/env python3
"""Record/verify the native table row-height corpus against Chromium via CDP."""
import argparse
import asyncio
import importlib.util
import json
from pathlib import Path
import sys

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "moli-benchmark"))
spec = importlib.util.spec_from_file_location(
    "table_height_oracle", ROOT / "scripts/layout-phase4-chromium-differential.py"
)
oracle = importlib.util.module_from_spec(spec)
sys.modules[spec.name] = oracle
spec.loader.exec_module(oracle)
FIXTURES = ROOT / "moli-renderer-v8/tests/fixtures"
EXPECTED = FIXTURES / "table-row-heights.chromium.json"


async def measure(client, session_id, case, *, record):
    _, messages = await oracle.command(client, "Page.navigate", {
        "url": "data:text/html;charset=utf-8," + oracle.urllib.parse.quote(case.html)
    }, session_id=session_id)
    await oracle.wait_for_load(client, session_id, messages)
    phases = []
    for phase in range(3):
        result, _ = await oracle.command(client, "Runtime.evaluate", {
            "expression": f"setTableRowHeightPhase({phase});collectTableRowHeights()",
            "returnByValue": True,
        }, session_id=session_id)
        if result.get("exceptionDetails"):
            raise RuntimeError(result["exceptionDetails"])
        phases.append(result["result"]["value"])
    if not record:
        expected = json.loads(EXPECTED.read_text())["phases"]
        for phase, (actual_cases, expected_cases) in enumerate(zip(phases, expected, strict=True)):
            for actual, expected_case in zip(actual_cases, expected_cases, strict=True):
                assert actual["name"] == expected_case["name"]
                oracle.assert_rects(f"phase {phase}: {actual['name']}", actual["rects"], expected_case["rects"])
    return phases


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--chromium", type=Path, default=oracle.DEFAULT_CHROMIUM)
    parser.add_argument("--record", action="store_true")
    args = parser.parse_args()
    html = (FIXTURES / "table-row-heights.html").read_text()
    oracle.fixture_cases = lambda: (oracle.Case("table-row-heights", 800, 600, html, ()),)
    oracle.measure_case = measure
    report = asyncio.run(oracle.run(args.chromium, record=args.record))
    phases = report["cases"]["table-row-heights"]
    if args.record:
        # One case per line keeps the numeric oracle compact and reviewable.
        header = json.dumps({"product": report["product"], "revision": report["revision"]}, indent=2)[:-2]
        body = ",\n".join("    [\n" + ",\n".join("      " + json.dumps(case) for case in phase) + "\n    ]" for phase in phases)
        EXPECTED.write_text(header + ',\n  "phases": [\n' + body + '\n  ]\n}\n')
    print(json.dumps({"status": report["status"], "product": report["product"], "cases": len(phases[0]), "phases": len(phases)}))


if __name__ == "__main__":
    main()
