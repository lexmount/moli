#!/usr/bin/env python3
"""Compare semantic dumps with alternating before/after release binaries.

Uses generated local fixtures, Linux wait4 peak RSS, three measured repetitions
by default, and SHA-256 checks for every output. Run without concurrent builds.
Example:
    python3 scripts/benchmark-ax-compact.py --before /path/before/moli \
        --after /path/after/moli --output target/ax-compact-benchmark/results
"""

import argparse
import functools
import hashlib
import html
import http.server
import json
import multiprocessing
import os
from pathlib import Path
import platform
import statistics
import subprocess
import threading
import time


def fixtures(root):
    root.mkdir(parents=True, exist_ok=True)

    def page(name, body):
        (root / f"{name}.html").write_text(
            f"<!doctype html><html><head><title>{name}</title></head>"
            f"<body>{body}</body></html>", encoding="utf-8"
        )

    for count in (500, 40000, 200000):
        page(f"rows-{count}", "".join(
            f'<div class="row"><a href="/item/{i}">Item {i}</a>'
            f'<span> description {i}</span></div>' for i in range(count)
        ))
    page("table-10000", '<table><caption>Results</caption><thead><tr>'
         '<th>Item</th><th>Count</th><th>Status</th></tr></thead><tbody>' + "".join(
             f'<tr><th scope="row">Item {i}</th><td>{i}</td><td>Ready</td></tr>'
             for i in range(10000)
         ) + '</tbody></table>')
    page("forms-300", "".join(
        f'<label for="t{i}">Name {i}</label><input id="t{i}" value="Value {i}">'
        f'<textarea aria-label="Notes {i}">Notes {i}</textarea>'
        f'<select aria-label="Choice {i}"><option>First</option>'
        '<option selected>Second</option></select>'
        f'<input type="checkbox" aria-label="Enabled {i}" checked>'
        f'<input type="range" aria-label="Amount {i}" value="42">'
        for i in range(300)
    ))
    page("ignored-10000", "".join(
        f'<div {attribute}><button>Ignored {i}</button><span>Text {i}</span></div>'
        for i in range(10000)
        for attribute in [('aria-hidden="true"', 'inert',
                           'style="visibility:hidden"')[i % 3]]
    ) + '<p>Visible sibling</p>')
    page("markers-10000", '<ul>' + "".join(
        f'<li>Bullet {i}</li>' for i in range(5000)
    ) + '</ul><ol start="9">' + "".join(
        f'<li>Number {i}</li>' for i in range(5000)
    ) + '</ol>')
    child = '<h1>Frame heading</h1>' + "".join(
        f'<p>Frame row {i}</p>' for i in range(200)
    ) + '<iframe srcdoc="&lt;button&gt;Nested action&lt;/button&gt;"></iframe>'
    page("frames", '<h1>Main heading</h1>' + "".join(
        f'<iframe title="Frame {i}" srcdoc="{html.escape(child, quote=True)}"></iframe>'
        for i in range(4)
    ))
    page("mixed", '<ul><li>Bullet</li></ul><ol start="9"><li>Number</li></ol>'
         '<input aria-label="Text" value="a value"><button disabled>Disabled</button>'
         '<div aria-hidden="true"><button>ARIA hidden</button></div>'
         '<div inert>Inert</div><div style="visibility:hidden">Invisible</div>'
         '<div hidden>Boolean hidden</div><div hidden="until-found">Until found</div>'
         '<div style="content-visibility:hidden">Locked</div><div id="host"></div>'
         '<iframe srcdoc="&lt;h1&gt;Child heading&lt;/h1&gt;"></iframe>'
         '<script>host.attachShadow({mode:"open"}).innerHTML="<button>Shadow</button>"</script>')


def run(binary, url, dump, frames, destination, timeout, env):
    args = ["moli", "fetch", "--timeout", "120000", "--dump", dump]
    if frames:
        args.append("--with-frames")
    args.append(url)
    started = time.monotonic()
    with destination.open("wb") as out, destination.with_suffix(".stderr").open("wb") as err:
        process = subprocess.Popen(args, executable=binary, stdout=out, stderr=err, env=env)
        timed_out = threading.Event()

        def terminate():
            timed_out.set()
            process.kill()

        watchdog = threading.Timer(timeout, terminate)
        watchdog.daemon = True
        watchdog.start()
        try:
            _, status, usage = os.wait4(process.pid, 0)
            process.returncode = os.waitstatus_to_exitcode(status)
        finally:
            watchdog.cancel()
    elapsed = time.monotonic() - started
    if timed_out.is_set():
        raise TimeoutError(f"{binary}: {url} {dump}")
    if process.returncode:
        raise RuntimeError(destination.with_suffix(".stderr").read_text())
    with destination.open("rb") as output:
        digest = hashlib.file_digest(output, "sha256").hexdigest()
    return {"maxrss_bytes": usage.ru_maxrss * 1024, "wall_seconds": elapsed,
            "output_bytes": destination.stat().st_size, "sha256": digest}


def distribution(path):
    with path.open() as file:
        nodes = json.load(file)
    return {
        "nodes": len(nodes),
        "ignored_reasons": sum("ignoredReasons" in node for node in nodes),
        "values": sum("value" in node for node in nodes),
        "nonempty_properties": sum(bool(node.get("properties")) for node in nodes),
        "rare_payloads": sum("ignoredReasons" in node or "value" in node
                             or bool(node.get("properties")) for node in nodes),
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--before", type=Path, required=True)
    parser.add_argument("--after", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--repeats", type=int, default=3)
    parser.add_argument("--timeout", type=float, default=180)
    parser.add_argument("--cases", nargs="+")
    parser.add_argument("--resume", action="store_true",
                        help="reuse complete samples after verifying binary hashes")
    args = parser.parse_args()
    if platform.system() != "Linux":
        parser.error("peak RSS reporting currently requires Linux")
    if args.repeats < 1:
        parser.error("--repeats must be positive")
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=True)
    root = output / "fixtures"
    # A large fixture's temporary strings must not inflate the RSS inherited
    # by subsequently forked benchmark processes.
    generator = multiprocessing.get_context("spawn").Process(target=fixtures, args=(root,))
    generator.start()
    generator.join()
    if generator.exitcode:
        raise RuntimeError("fixture generation failed")
    binaries = {"before": str(args.before.resolve()), "after": str(args.after.resolve())}
    manifest = {"binaries": {}, "platform": platform.platform(), "repeats": args.repeats}
    for label, binary in binaries.items():
        with open(binary, "rb") as file:
            manifest["binaries"][label] = {
                "path": binary, "sha256": hashlib.file_digest(file, "sha256").hexdigest()
            }
    if args.resume:
        previous_manifest = json.loads((output / "manifest.json").read_text())
        if previous_manifest != manifest:
            parser.error("resume requires the same binaries, platform and repeat count")
    (output / "manifest.json").write_text(json.dumps(manifest, indent=2) + "\n")
    env = {key: value for key, value in os.environ.items()
           if key.lower() not in ("http_proxy", "https_proxy", "all_proxy")}

    class Handler(http.server.SimpleHTTPRequestHandler):
        def log_message(self, *unused):
            pass

    server = http.server.ThreadingHTTPServer(
        ("127.0.0.1", 0), functools.partial(Handler, directory=str(root))
    )
    threading.Thread(target=server.serve_forever, daemon=True).start()
    base = f"http://127.0.0.1:{server.server_port}"
    cases = ["rows-500", "rows-40000", "table-10000", "forms-300",
             "ignored-10000", "markers-10000", "frames", "mixed", "rows-200000"]
    if args.cases:
        unknown = set(args.cases) - set(cases)
        if unknown:
            parser.error(f"unknown cases: {sorted(unknown)}")
        cases = args.cases
    records = []
    completed = set()
    if args.resume:
        previous = [json.loads(line) for line in (output / "results.jsonl").read_text().splitlines()]
        expected_trials = {(label, repeat) for label in binaries for repeat in range(args.repeats)}
        for case, dump in dict.fromkeys((row["case"], row["dump"]) for row in previous):
            group = [row for row in previous if (row["case"], row["dump"]) == (case, dump)]
            trials = {(row["binary"], row["repeat"]) for row in group}
            if trials == expected_trials and len(group) == len(expected_trials):
                if len({row["sha256"] for row in group}) != 1:
                    raise AssertionError(f"existing output mismatch: {case} {dump}")
                completed.add((case, dump))
        records = [row for row in previous if (row["case"], row["dump"]) in completed]
    try:
        # Both binaries receive the same argv[0], URL, environment and cwd.
        for label, binary in binaries.items():
            run(binary, base + "/rows-500.html", "semantic_tree_text", False,
                output / f"warmup-{label}.out", args.timeout, env)
        with (output / "results.jsonl").open("w") as log:
            for record in records:
                log.write(json.dumps(record) + "\n")
            log.flush()
            for case in cases:
                dumps = ("semantic_tree_text",) if case == "rows-200000" else (
                    "semantic_tree_text", "semantic_tree"
                )
                if case == "rows-40000":
                    dumps += ("markdown",)
                for dump in dumps:
                    if (case, dump) in completed:
                        print(f"Reusing complete samples: {case} {dump}", flush=True)
                        continue
                    hashes = set()
                    for repeat in range(args.repeats):
                        order = ("before", "after") if repeat % 2 == 0 else ("after", "before")
                        for label in order:
                            destination = output / f"{case}-{dump}-{repeat}-{label}.out"
                            record = {"case": case, "dump": dump, "repeat": repeat, "binary": label,
                                      **run(binaries[label], base + f"/{case}.html", dump,
                                            case in ("frames", "mixed"), destination, args.timeout, env)}
                            hashes.add(record["sha256"])
                            records.append(record)
                            line = json.dumps(record)
                            log.write(line + "\n")
                            log.flush()
                            print(line, flush=True)
                    if len(hashes) != 1:
                        raise AssertionError(f"output mismatch: {case} {dump}")
    finally:
        server.shutdown()
        server.server_close()

    lines = ["# AX compact benchmark", "",
             "Generated local fixtures; alternating runs; median of "
             f"{args.repeats} runs; RSS in MiB. Before is the existing typed AX implementation.", "",
             "| Case | Dump | RSS before | RSS after | RSS change | Time before | Time after | Time change |",
             "|---|---|---:|---:|---:|---:|---:|---:|"]
    for case, dump in dict.fromkeys((row["case"], row["dump"]) for row in records):
        stats = {}
        for label in binaries:
            rows = [row for row in records if (row["case"], row["dump"], row["binary"]) == (case, dump, label)]
            stats[label] = {key: statistics.median(row[key] for row in rows)
                            for key in ("maxrss_bytes", "wall_seconds")}
        before, after = stats["before"], stats["after"]
        rss_change = (after["maxrss_bytes"] / before["maxrss_bytes"] - 1) * 100
        wall_change = (after["wall_seconds"] / before["wall_seconds"] - 1) * 100
        lines.append(f"| {case} | {dump} | {before['maxrss_bytes']/2**20:.1f} | "
                     f"{after['maxrss_bytes']/2**20:.1f} | {rss_change:+.1f}% | "
                     f"{before['wall_seconds']:.3f}s | {after['wall_seconds']:.3f}s | {wall_change:+.1f}% |")
    lines += ["", "Every output is byte-for-byte identical across binaries and repetitions.", "",
              "Small-page timings include process startup and are sensitive to noise.", "",
              "| Case | AX nodes | Ignored reasons | Values | Nonempty properties | Nodes with rare payload |",
              "|---|---:|---:|---:|---:|---:|"]
    distributions = {}
    for case in cases:
        path = output / f"{case}-semantic_tree-0-after.out"
        if not path.exists():
            continue
        stats = distributions[case] = distribution(path)
        lines.append(f"| {case} | {stats['nodes']} | {stats['ignored_reasons']} | "
                     f"{stats['values']} | {stats['nonempty_properties']} | "
                     f"{stats['rare_payloads'] / max(stats['nodes'], 1):.1%} |")
    (output / "node-distribution.json").write_text(json.dumps(distributions, indent=2) + "\n")
    lines += ["", "Raw samples and binary hashes: `results.jsonl`, `manifest.json`.", ""]
    report = "\n".join(lines)
    (output / "report.md").write_text(report)
    print(report)


if __name__ == "__main__":
    main()
