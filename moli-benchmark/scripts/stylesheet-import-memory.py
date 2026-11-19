#!/usr/bin/env python3
"""Compare Release builds on duplicate linked stylesheets and a wide import graph.

    python3 moli-benchmark/scripts/stylesheet-import-memory.py \
        --before /tmp/moli-before --after /tmp/moli-after --count-operations

Requires Unix wait4. The optional operation counts require LLDB and binaries
with symbols. Debugger runs are separate from time and RSS measurements.
"""

import argparse
import http.server
import json
import os
from pathlib import Path
import re
import subprocess
import sys
import tempfile
import threading
import time


RULE_COUNT = 6000
LINK_COUNT = 12
IMPORT_COUNT = 24


def stylesheet_fixtures():
    css = "".join(
        f".tile-{i} {{ color: rgb({i % 255}, 31, 97); padding: {i % 16}px; "
        "margin: 3px 6px; display: block; }\n"
        for i in range(RULE_COUNT)
    )
    repeated_links = '<link rel="stylesheet" href="/large.css">' * LINK_COUNT
    imports = "".join(f'@import "/leaf-{i}.css";\n' for i in range(IMPORT_COUNT))
    html = "text/html; charset=utf-8"
    css_mime = "text/css; charset=utf-8"
    fixtures = {
        "/duplicate-links": (html, '<!doctype html><iframe src="/links.html"></iframe>'),
        "/links.html": (html, "<!doctype html>" + repeated_links),
        "/import-fanout": (html, '<!doctype html><link rel="stylesheet" href="/root.css">'),
        "/root.css": (css_mime, imports),
        "/large.css": (css_mime, css),
    }
    fixtures.update({f"/leaf-{i}.css": (css_mime, css) for i in range(IMPORT_COUNT)})
    return {path: (mime, text.encode()) for path, (mime, text) in fixtures.items()}


class FixtureHandler(http.server.BaseHTTPRequestHandler):
    fixtures = stylesheet_fixtures()

    def do_GET(self):
        response = self.fixtures.get(self.path)
        if response is None:
            self.send_error(404)
            return
        mime, body = response
        self.send_response(200)
        self.send_header("Content-Type", mime)
        self.send_header("Content-Length", str(len(body)))
        self.send_header("Cache-Control", "no-store")
        self.end_headers()
        self.wfile.write(body)

    def log_message(self, *_args):
        pass


CASES = {
    "duplicate-links": (
        """(() => {
            const links = document.querySelector('iframe').contentDocument.querySelectorAll('link');
            const sheets = Array.from(links, link => link.sheet);
            return {owners: new Set(sheets).size, rules: sheets.map(sheet => sheet.cssRules.length)};
        })()""",
        {"owners": LINK_COUNT, "rules": [RULE_COUNT] * LINK_COUNT},
    ),
    "import-fanout": (
        """(() => {
            const rules = document.querySelector('link').sheet.cssRules;
            const sheets = Array.from(rules, rule => rule.styleSheet);
            return {owners: new Set(sheets).size, rules: sheets.map(sheet => sheet.cssRules.length)};
        })()""",
        {"owners": IMPORT_COUNT, "rules": [RULE_COUNT] * IMPORT_COUNT},
    ),
}


def fetch_command(binary, origin, case):
    expression, _ = CASES[case]
    expression = " ".join(line.strip() for line in expression.splitlines())
    return [
        str(binary), "fetch", "--wait", "load", "--timeout-ms", "90000",
        "--http-no-proxy", "127.0.0.1", "--eval", expression, f"{origin}/{case}",
    ]


def measure(command, expected):
    started = time.monotonic()
    with tempfile.TemporaryFile() as output, tempfile.TemporaryFile() as errors:
        with subprocess.Popen(command, stdout=output, stderr=errors) as child:
            _, status, usage = os.wait4(child.pid, 0)
            child.returncode = os.waitstatus_to_exitcode(status)
        elapsed_ms = (time.monotonic() - started) * 1000
        output.seek(0)
        errors.seek(0)
        if child.returncode:
            raise RuntimeError(errors.read().decode(errors="replace"))
        assert_result(output.read().decode(), expected)
    return {
        "elapsed_ms": elapsed_ms,
        "peak_rss_bytes": usage.ru_maxrss * (1 if sys.platform == "darwin" else 1024),
    }


def assert_result(output, expected):
    for line in output.splitlines():
        if line.startswith("{") and json.loads(line) == expected:
            return
    raise RuntimeError(f"stylesheet fixture did not finish with the expected CSSOM\n{output}")


def count_operations(command, expected):
    symbols = [
        "<style::stylesheets::stylesheet::StylesheetContents>::from_str",
        "<style::stylesheets::stylesheet::StylesheetContents>::deep_clone",
    ]
    debugger = ["lldb", "--batch", "--no-lldbinit", "-o", "settings set target.disable-aslr false"]
    for symbol in symbols:
        debugger.extend(["-o", f'breakpoint set -n "{symbol}"'])
    debugger.extend([
        "-o", "breakpoint modify --auto-continue true 1 2",
        "-o", "run", "-o", "breakpoint list", "--",
    ])
    result = subprocess.run(
        debugger + command, text=True, capture_output=True, timeout=150,
    )
    if result.returncode:
        raise RuntimeError(f"LLDB run failed\n{result.stdout}{result.stderr}")
    assert_result(result.stdout, expected)
    counts = re.findall(r"^[12]: name = .*resolved = [1-9]\d*, hit count = (\d+)", result.stdout, re.M)
    if len(counts) != 2:
        raise RuntimeError(f"stylesheet symbols could not be counted\n{result.stdout}")
    return {"stylesheet_parses": int(counts[0]), "stylesheet_contents_clones": int(counts[1])}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--before", type=Path, required=True)
    parser.add_argument("--after", type=Path, required=True)
    parser.add_argument("--runs", type=int, default=7)
    parser.add_argument("--count-operations", action="store_true")
    args = parser.parse_args()
    if args.runs < 1:
        parser.error("--runs must be positive")
    binaries = [("before", args.before.resolve()), ("after", args.after.resolve())]
    with http.server.ThreadingHTTPServer(("127.0.0.1", 0), FixtureHandler) as server:
        origin = f"http://127.0.0.1:{server.server_port}"
        thread = threading.Thread(target=server.serve_forever, daemon=True)
        thread.start()
        try:
            for case, (_, expected) in CASES.items():
                for run in range(args.runs):
                    order = binaries if run % 2 == 0 else reversed(binaries)
                    for label, binary in order:
                        command = fetch_command(binary, origin, case)
                        metrics = measure(command, expected)
                        print(json.dumps(dict(case=case, binary=label, run=run, **metrics)), flush=True)
                if args.count_operations:
                    for label, binary in binaries:
                        counts = count_operations(fetch_command(binary, origin, case), expected)
                        print(json.dumps(dict(case=case, binary=label, **counts)), flush=True)
        finally:
            server.shutdown()
            thread.join()


if __name__ == "__main__":
    main()
