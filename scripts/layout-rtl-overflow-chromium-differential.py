#!/usr/bin/env python3
"""Compare physical RTL overflow geometry and scrolling against live Chromium.

uv run --project moli-benchmark python \
  scripts/layout-rtl-overflow-chromium-differential.py --moli-bin target/release/moli

Runs with hidden and native scrollbars. Every round navigates fresh documents;
scroll writes and geometry reads cross rendering/task boundaries. A mismatch
is an error, including when both engines would otherwise match an old snapshot.
"""

from __future__ import annotations

import argparse
import asyncio
import contextlib
import hashlib
import importlib.util
import itertools
import json
import os
import shutil
import subprocess
import sys
import tempfile
import time
import urllib.parse
from pathlib import Path
from typing import Any

ROOT = Path(__file__).resolve().parents[1]
SPEC = importlib.util.spec_from_file_location(
    "rtl_overflow_cdp", ROOT / "scripts/layout-phase5-chromium-differential.py"
)
assert SPEC is not None and SPEC.loader is not None
cdp = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = cdp
SPEC.loader.exec_module(cdp)


def fixture_cases() -> dict[str, str]:
    cases = {}
    for direction, display, shape in itertools.product(
        ("ltr", "rtl"), ("block", "flex", "grid"),
        ("fit", "wide", "nested-ltr", "nested-rtl", "translated-positive", "translated-negative", "scaled", "scaled-border"),
    ):
        wide = shape != "fit"
        # Unequal physical edges detect mirroring, axis swaps, and
        # adding the leading scrollbar gutter a second time.
        css = f"""
html,body{{margin:0;padding:0}}
#scroller{{position:absolute;left:40px;top:30px;width:120px;height:90px;
border-style:solid;border-width:2px 5px 13px 19px;padding:3px 11px 7px 17px;
overflow:auto;direction:{direction};display:{display}}}
#child{{flex:none;min-width:0;min-height:0;width:{260 if wide else 40}px;
height:{180 if wide else 30}px}}
"""
        inner = ""
        if shape.startswith("nested-"):
            inner_direction = shape.removeprefix("nested-")
            css += f"""
#child{{width:80px;height:50px;direction:{inner_direction};overflow:visible}}
#descendant{{width:260px;height:180px}}
"""
            inner = '<div id="descendant"></div>'
        if display == "grid":
            css += "#scroller{grid-template-columns:80px;grid-template-rows:50px}"
        transforms = {"translated-positive": "translate(30px,20px)",
                      "translated-negative": "translate(-30px,-20px)",
                      "scaled": "scale(2)", "scaled-border": "scale(2)"}
        if shape in transforms:
            css += f"#child{{transform:{transforms[shape]}}}"
        if shape == "scaled-border":
            css += "#child{border-style:solid;border-width:2px 4px 6px 8px}"
        cases[f"horizontal-tb/{direction}-{display}-{shape}"] = (
            f'<!doctype html><style>{css}</style>'
            f'<div id="scroller"><div id="child">{inner}</div></div>'
        )
    # Ordinary vertical block flow also needs to anchor RTL at physical bottom;
    # otherwise correct overflow reachability can hide a necessary scrollbar.
    for mode, direction, shape in itertools.product(
        ("vertical-rl", "vertical-lr"), ("ltr", "rtl"), ("fit", "wide", "relative-margin"),
    ):
        wide = shape != "fit"
        extra = "position:relative;top:9px;margin:7px 5px 11px 13px" if shape == "relative-margin" else ""
        cases[f"{mode}/{direction}-block-flow-{shape}"] = f"""<!doctype html><style>
html,body{{margin:0;padding:0}}
#scroller{{position:absolute;left:40px;top:30px;width:120px;height:90px;
border-style:solid;border-width:2px 5px 13px 19px;padding:3px 11px 7px 17px;
overflow:auto;writing-mode:{mode};direction:{direction}}}
#child{{width:{260 if wide else 40}px;height:{180 if wide else 30}px;{extra}}}
</style><div id="scroller"><div id="child"></div></div>"""
    # Explicit physical insets isolate vertical scroll-origin conversion from
    # the separate normal-flow/grid writing-mode layout algorithms. Exercise
    # overflow on both physical axes, including negative reachable extents.
    shapes = (
        ("fit", 20, 10, 40, 30),
        ("positive", 20, 10, 260, 180),
        ("negative-x", -160, 10, 260, 180),
        ("negative-y", 20, -130, 260, 180),
        ("negative-both", -160, -130, 260, 180),
        ("end-corner", 80, 60, 40, 30),
    )
    for mode, direction, (shape, x, y, width, height) in itertools.product(
        ("vertical-rl", "vertical-lr"), ("ltr", "rtl"), shapes,
    ):
        cases[f"{mode}/{direction}-positioned-{shape}"] = f"""<!doctype html><style>
html,body{{margin:0;padding:0}}
#scroller{{position:absolute;left:40px;top:30px;width:120px;height:90px;
border-style:solid;border-width:2px 5px 13px 19px;padding:3px 11px 7px 17px;
overflow:auto;writing-mode:{mode};direction:{direction}}}
#child{{position:absolute;left:{x}px;top:{y}px;width:{width}px;height:{height}px}}
</style><div id="scroller"><div id="child"></div></div>"""
    for direction, clip in itertools.product(("ltr", "rtl"), ("x", "y")):
        cases[f"horizontal-tb/{direction}-clip-{clip}-visible"] = f"""<!doctype html><style>
html,body{{margin:0;padding:0}}
#scroller{{width:100px;height:80px;overflow:auto;direction:{direction}}}
#child{{width:80px;height:50px;overflow-{clip}:clip}}
#descendant{{width:260px;height:180px}}
</style><div id="scroller"><div id="child"><div id="descendant"></div></div></div>"""
    for mode, direction, flex, wrap in itertools.product(
        ("horizontal-tb", "vertical-rl", "vertical-lr"), ("ltr", "rtl"),
        ("row", "row-reverse", "column", "column-reverse"), ("nowrap", "wrap-reverse"),
    ):
        cases[f"{mode}/{direction}-flow-{flex}-{wrap}"] = f"""<!doctype html><style>
html,body{{margin:0;padding:0}}
#scroller{{display:flex;width:120px;height:90px;overflow:auto;
writing-mode:{mode};direction:{direction};flex-direction:{flex};flex-wrap:{wrap};
align-items:flex-start;align-content:flex-start}}
#child{{flex:none;width:260px;height:180px}}
</style><div id="scroller"><div id="child"></div></div>"""
    for direction, empty in itertools.product(("ltr", "rtl"), (False, True)):
        child = "display:none" if empty else "width:40px;height:30px"
        cases[f"horizontal-tb/{direction}-grid-area-{'empty' if empty else 'small-item'}"] = f"""<!doctype html><style>
html,body{{margin:0;padding:0}}
#scroller{{position:absolute;left:40px;top:30px;display:grid;width:120px;height:90px;
padding:3px 11px 7px 17px;border-style:solid;border-width:2px 5px 13px 19px;
overflow:auto;direction:{direction};grid-template-columns:260px;grid-template-rows:180px}}
#child{{{child}}}
</style><div id="scroller"><div id="child"></div></div>"""
    return cases


SNAPSHOT = """(() => {
  const s = document.getElementById('scroller');
  const rect = e => { const r=e.getBoundingClientRect();
    return [r.x,r.y,r.width,r.height]; };
  return {
    scroll: [s.scrollWidth,s.scrollHeight,s.scrollLeft,s.scrollTop],
    client: [s.clientLeft,s.clientTop,s.clientWidth,s.clientHeight],
    scroller: rect(s), child: rect(document.getElementById('child')),
    descendant: document.getElementById('descendant')
      ? rect(document.getElementById('descendant')) : null
  };
})()"""


async def evaluate(client, session: str, expression: str) -> Any:
    result, _ = await cdp.command(client, "Runtime.evaluate", {
        "expression": expression, "returnByValue": True, "awaitPromise": True,
    }, session_id=session)
    if result.get("exceptionDetails"):
        raise RuntimeError(result["exceptionDetails"])
    return result["result"].get("value")


async def settle(client, session: str) -> None:
    # Moli publishes layout at rendering boundaries. Reading inside the same
    # script as a scroll mutation would test its synchronous sampling contract.
    await evaluate(client, session, "new Promise(r => requestAnimationFrame(() => requestAnimationFrame(r)))")
    # getLayoutMetrics deliberately reuses Moli's last published frame. A
    # capture explicitly publishes fresh visual geometry after each mutation.
    await cdp.command(client, "Page.captureScreenshot", {
        "clip": {"x": 0, "y": 0, "width": 1, "height": 1, "scale": 1},
    }, session_id=session)


async def measure(client, session: str, html: str) -> dict[str, Any]:
    _, messages = await cdp.command(client, "Page.navigate", {
        "url": "data:text/html;charset=utf-8," + urllib.parse.quote(html),
    }, session_id=session)
    await cdp.wait_for_load(client, session, messages)
    await settle(client, session)
    values = {"initial": await evaluate(client, session, SNAPSHOT)}
    for name, x, y in (("negative", -10000, -10000), ("positive", 10000, 10000)):
        await evaluate(client, session, f"""(() => {{
          const s=document.getElementById('scroller');
          s.scrollLeft={x};s.scrollTop={y};
        }})()""")
        await settle(client, session)
        values[name] = await evaluate(client, session, SNAPSHOT)
    json.dumps(values, allow_nan=False)
    return values


async def run_engine(binary: Path, engine: str, scrollbars: bool, rounds: int,
                     output: Path) -> dict[str, Any]:
    port = cdp.reserve_port()
    endpoint = f"http://127.0.0.1:{port}"
    label = f"{engine}-{'native' if scrollbars else 'hidden'}"
    with tempfile.TemporaryDirectory(prefix=f"moli-rtl-{label}-", ignore_cleanup_errors=True) as profile:
        if engine == "moli":
            command = [str(binary), "serve", "--host", "127.0.0.1", "--port", str(port), "--layout"]
            if scrollbars:
                command.append("--scrollbars")
        else:
            command = [str(binary), "--headless=new", "--disable-gpu", "--no-sandbox",
                       "--disable-background-networking", "--no-first-run",
                       "--remote-debugging-address=127.0.0.1", f"--remote-debugging-port={port}",
                       f"--user-data-dir={profile}", "about:blank"]
            if not scrollbars:
                command.append("--hide-scrollbars")
        env = {k: v for k, v in os.environ.items()
               if k.lower() not in {"http_proxy", "https_proxy", "all_proxy", "no_proxy",
                                    "moli_scrollbars", "moli_layout"}}
        with (output / f"{label}.log").open("w") as log:
            process = subprocess.Popen(command, stdout=log, stderr=subprocess.STDOUT, env=env)
            client = None
            try:
                deadline = time.monotonic() + 15
                while time.monotonic() < deadline:
                    if process.poll() is not None:
                        raise RuntimeError(f"{label} exited: {process.returncode}; see {log.name}")
                    try:
                        client = await cdp.connect_raw_cdp(endpoint)
                        break
                    except (OSError, TimeoutError, ValueError):
                        await asyncio.sleep(0.05)
                if client is None:
                    raise TimeoutError(f"{label}: CDP startup timed out")
                version, _ = await cdp.command(client, "Browser.getVersion")
                target, _ = await cdp.command(client, "Target.createTarget", {"url": "about:blank"})
                attached, _ = await cdp.command(client, "Target.attachToTarget", {
                    "targetId": target["targetId"], "flatten": True,
                })
                session = attached["sessionId"]
                await cdp.command(client, "Page.enable", session_id=session)
                await cdp.command(client, "Emulation.setDeviceMetricsOverride", {
                    "width": 800, "height": 600, "deviceScaleFactor": 1, "mobile": False,
                }, session_id=session)
                observations = []
                for number in range(1, rounds + 1):
                    cases = {name: await measure(client, session, html)
                             for name, html in fixture_cases().items()}
                    observations.append(cases)
                    print(f"{label}: round {number}, {len(cases)} cases", flush=True)
                return {"command": command, "version": version,
                        "binary_sha256": hashlib.sha256(binary.read_bytes()).hexdigest(),
                        "rounds": observations}
            finally:
                if client is not None:
                    with contextlib.suppress(Exception):
                        await cdp.command(client, "Browser.close")
                    with contextlib.suppress(Exception):
                        await client.websocket.close()
                process.terminate()
                try:
                    process.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    process.kill()
                    process.wait(timeout=5)


async def run(args, output: Path) -> bool:
    report: dict[str, Any] = {"tolerance_css_px": cdp.TOLERANCE,
                              "fixtures": fixture_cases(), "engines": {}, "differences": []}
    for scrollbars in (False, True):
        mode = "native" if scrollbars else "hidden"
        expected = await run_engine(args.chromium, "chromium", scrollbars, args.rounds, output)
        report["engines"][f"chromium-{mode}"] = expected
        (output / "comparison.json").write_text(json.dumps(report, indent=2) + "\n")
        actual = await run_engine(args.moli_bin, "moli", scrollbars, args.rounds, output)
        report["engines"][f"moli-{mode}"] = actual
        for number, (moli_round, chrome_round) in enumerate(zip(actual["rounds"], expected["rounds"]), 1):
            for name, oracle in chrome_round.items():
                try:
                    cdp.assert_value(f"{mode}/{name}", moli_round[name], oracle)
                except AssertionError as error:
                    report["differences"].append({"mode": mode, "case": name, "round": number,
                                                  "error": str(error)})
        (output / "comparison.json").write_text(json.dumps(report, indent=2) + "\n")
    report["passed"] = not report["differences"]
    report["complete"] = True
    (output / "comparison.json").write_text(json.dumps(report, indent=2) + "\n")
    print(json.dumps({"passed": report["passed"], "differences": report["differences"],
                      "report": str(output / "comparison.json")}, indent=2))
    return report["passed"]


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--moli-bin", type=Path, required=True)
    parser.add_argument("--chromium", type=Path, default=Path(shutil.which("chromium") or cdp.DEFAULT_CHROMIUM))
    parser.add_argument("--rounds", type=int, default=3)
    parser.add_argument("--output-dir", type=Path)
    args = parser.parse_args()
    if args.rounds < 1:
        parser.error("--rounds must be positive")
    args.moli_bin = args.moli_bin.expanduser().resolve(strict=True)
    args.chromium = args.chromium.expanduser().resolve(strict=True)
    output = args.output_dir or ROOT / "moli-benchmark/results/rtl-overflow" / time.strftime("%Y%m%d-%H%M%S")
    output.mkdir(parents=True, exist_ok=False)
    raise SystemExit(0 if asyncio.run(run(args, output)) else 1)


if __name__ == "__main__":
    main()
