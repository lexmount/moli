"""Measure batched inline style writes in Moli's realtime layout mode.

Pass equally built binaries to compare revisions. Each sample starts a fresh
document and checks that the final geometry reflects the writes.

Usage: python3 moli-benchmark/scripts/probe-style-invalidation-queue.py BASE CANDIDATE
"""

from __future__ import annotations

import argparse
import json
import subprocess
from pathlib import Path
from statistics import median
from urllib.parse import quote


FIXTURE_URL = "data:text/html," + quote('<!doctype html><div id="probe"></div>')
EXPRESSION = """(() => {
    const parent = document.getElementById("probe");
    void parent.offsetWidth;
    const started = performance.now();
    for (let index = 0; index < NODE_COUNT; index++) {
        const node = document.createElement("span");
        node.textContent = "mmmmmmmmmmlli";
        parent.appendChild(node);
        node.style.position = "absolute";
        node.style.fontSize = "20px";
        node.style.fontFamily = index % 2 ? "monospace" : "serif";
        node.style.width = "30px";
    }
    const written = performance.now();
    const width = parent.lastElementChild.offsetWidth;
    return {
        children: parent.childElementCount, width,
        writesMs: written - started,
        readMs: performance.now() - written,
    };
})()"""


def measure(binary: Path, count: int) -> tuple[float, float]:
    result = subprocess.run(
        [
            str(binary),
            "fetch",
            "--realtime",
            "--wait-until",
            "load",
            "--eval",
            EXPRESSION.replace("NODE_COUNT", str(count)),
            FIXTURE_URL,
        ],
        capture_output=True,
        text=True,
        timeout=60,
    )
    if result.returncode:
        raise RuntimeError(f"{binary} exited {result.returncode}\n{result.stderr}")
    sample = json.loads(result.stdout)
    if sample["children"] != count or sample["width"] != 30:
        raise RuntimeError(
            f"unexpected geometry for {binary} with {count} nodes: {sample}"
        )
    return sample["writesMs"], sample["readMs"]


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("binaries", type=Path, nargs="+")
    parser.add_argument("--nodes", type=int, nargs="+", default=[500, 1000, 2000])
    parser.add_argument("--runs", type=int, default=3)
    args = parser.parse_args()
    if args.runs < 1 or any(count < 1 for count in args.nodes):
        parser.error("--runs and --nodes require positive counts")
    binaries = [path.resolve(strict=True) for path in args.binaries]

    print("nodes  writes_ms  layout_read_ms  binary", flush=True)
    for count in args.nodes:
        samples: list[list[tuple[float, float]]] = [[] for _ in binaries]
        for run in range(args.runs):
            order = range(len(binaries))
            for index in order if run % 2 == 0 else reversed(order):
                samples[index].append(measure(binaries[index], count))
        for binary, measurements in zip(binaries, samples, strict=True):
            writes, reads = zip(*measurements, strict=True)
            print(
                f"{count:5d}  {median(writes):9.1f}  {median(reads):14.1f}  {binary}",
                flush=True,
            )


if __name__ == "__main__":
    main()
