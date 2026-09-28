from __future__ import annotations

import argparse
import asyncio
from datetime import datetime, timezone
from pathlib import Path

from ..config import RESULTS_ROOT, moli_binary, optional_binary
from .cases import CASES, select_cases
from .runner import run_suite


def add_arguments(parser: argparse.ArgumentParser) -> None:
    parser.add_argument("--site", action="append", choices=[case.id for case in CASES])
    parser.add_argument("--engine", action="append", choices=["moli", "chromium"])
    parser.add_argument("--moli-bin")
    parser.add_argument("--chrome-bin")
    parser.add_argument("--runs", type=int, default=1, help="predeclared repetitions, never retries; 1..10")
    parser.add_argument("--output-dir", type=Path, default=None)


def command(args) -> int:
    engines = args.engine or ["moli", "chromium"]
    if len(set(engines)) != len(engines):
        raise ValueError("engine selection must be unique")
    cases = select_cases(args.site)
    binaries = {}
    for engine in engines:
        binary = moli_binary(args.moli_bin) if engine == "moli" else optional_binary(
            "CHROME_BIN", ("chromium", "chromium-browser", "google-chrome"), args.chrome_bin)
        if binary is None:
            raise ValueError("native Chromium is required; supply --chrome-bin (no bundled browser is launched)")
        binaries[engine] = binary
    output = args.output_dir or RESULTS_ROOT / ("fingerprint-" + datetime.now(timezone.utc).strftime("%Y%m%dT%H%M%S.%fZ"))
    summary = asyncio.run(run_suite(output, binaries, cases, runs=args.runs))
    print(output)
    # A site's bot verdict never determines this process exit code. An incomplete
    # experiment or a broken collector still cannot silently return success.
    harness_errors = {"collector_error", "browser_crash", "cleanup_error"}
    return int(not summary["matrix_complete"] or not summary["inputs_unchanged"]
               or any(row["status"] in harness_errors for row in summary["suites"][0]["samples"]))


def main() -> int:
    parser = argparse.ArgumentParser(description="Native CDP twelve-site observations; availability is not bot pass rate")
    add_arguments(parser)
    try:
        return command(parser.parse_args())
    except (RuntimeError, ValueError, FileExistsError, ImportError) as error:
        parser.exit(1, f"fingerprint benchmark: {error}\n")
    except KeyboardInterrupt:
        return 130
