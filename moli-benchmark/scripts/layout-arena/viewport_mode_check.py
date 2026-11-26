"""Compare desktop/mobile viewport transitions, root gutters and fixed boxes."""

from __future__ import annotations

import argparse
import asyncio
import json
from pathlib import Path
from types import SimpleNamespace
from urllib.parse import quote

from web_bench import SITES, one_run

VARIANTS = ("baseline", "slotmap_before", "dense_before", "slotmap", "dense")
HTML = """<!doctype html><title>Viewport transition fixture</title><style>
body{margin:0;font:16px/1.25 sans-serif}p{margin:0}#fluid{width:65vw;height:30vh}
#percent{width:55%;height:25%}#fixed{position:fixed;right:4vw;bottom:3vh}
#scroll{width:40vw;height:48px;overflow:auto}#scroll p{width:260px}
@media(max-width:600px){#fluid{padding:3px}#scroll{height:28px}}
@media(min-resolution:2dppx){html{font-size:19px}}</style>
<div id=fluid><div id=percent>percent</div></div><span id=fixed>fixed</span>
<div id=scroll><p>alpha beta gamma delta epsilon zeta eta theta iota kappa</p></div>
<div style="height:600px">tall</div>""" + "".join(
    f"<p id=p{i}>line {i} for root scrollbars</p>" for i in range(28)
)
OBSERVE = """(() => {const box=n=>{const r=n.getBoundingClientRect();return [r.x,r.y,r.width,r.height]};
return {viewport:[innerWidth,innerHeight,devicePixelRatio],visual:[visualViewport.width,visualViewport.height,visualViewport.scale],
root:[document.documentElement.clientWidth,document.documentElement.clientHeight,document.documentElement.scrollHeight,...box(document.documentElement)],
boxes:Array.from(document.querySelectorAll('body *'),n=>[n.id,...box(n)])}})()"""
CASES = [
    (1280, 2400, False, 1),
    (400, 700, True, 2),
    (480, 360, True, 2),
    (800, 420, False, 1),
    (1280, 2400, False, 1),
]


async def transition(client, proc, sampler):
    points = []
    for i, (width, height, mobile, scale) in enumerate(CASES):
        if i == 2:
            await client.evaluate(
                "const meta=document.createElement('meta');meta.id='viewport-meta';meta.name='viewport';meta.content='width=device-width, initial-scale=1';document.head.append(meta);true"
            )
        if i == 3:
            await client.evaluate(
                "document.getElementById('viewport-meta').remove();true"
            )
        await client.command(
            "Emulation.setDeviceMetricsOverride",
            {
                "width": width,
                "height": height,
                "deviceScaleFactor": scale,
                "mobile": mobile,
            },
        )
        await client.evaluate("document.fonts.ready.then(()=>true)")
        points.append(await client.evaluate(OBSERVE))
    assert points[0] == points[-1], "Initial geometry not restored"
    assert points[0]["root"][2] <= points[0]["viewport"][1], "Initial root should fit"
    assert points[3]["root"][2] > points[3]["viewport"][1], (
        "Short desktop root should overflow"
    )
    return points


async def run(args):
    root = args.binaries
    output = args.output
    output.mkdir(exist_ok=False)
    SITES["viewport_fixture"] = "data:text/html;charset=utf-8," + quote(HTML)
    reference = None
    rows = []
    for v in VARIANTS:
        folder = output / v
        options = SimpleNamespace(
            binary=root / (v + "-moli"),
            site="viewport_fixture",
            mode="live",
            archive=root / "unused-fixture-archive",
            output=folder,
            seconds=2,
            cpus="3,4",
            network_mode="direct",
            sample_interval=0.1,
            max_rss_mib=4096,
            idle_seconds=0,
            close_seconds=0,
            close_page=False,
            observe_only=False,
            load_timeout=30,
            scenario_runner=transition,
        )
        await one_run(options)
        result = json.loads((folder / "result.json").read_text())
        assert not result["navigation"].get("errorText"), "Navigation failed"
        assert result["observation"]["title"] == "Viewport transition fixture"
        assert result["observation"]["nodes"] >= 30, "Fixture did not initialize"
        points = result["layout_scenarios"]
        if reference is not None:
            assert points == reference, f"Viewport-mode geometry differs: {v}"
        reference = points
        rows.append(
            {
                "variant": v,
                "binary_sha256": result["binary_sha256"],
                "status": "ok",
                "observations": points,
            }
        )
        (output / "results.json").write_text(json.dumps(rows, indent=2))
    print(
        "5/5 desktop/mobile/meta viewport/root gutter/fixed-position parity checks passed"
    )


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binaries", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    asyncio.run(run(parser.parse_args()))
