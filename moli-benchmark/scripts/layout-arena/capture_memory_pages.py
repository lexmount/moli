"""Capture a public-page catalogue and screen snapshots before comparisons."""

from __future__ import annotations

import argparse
import asyncio
import json
import re
import time
from pathlib import Path
from types import SimpleNamespace

from web_bench import SITES, one_run


async def capture(args):
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=False)
    catalogue = json.loads(args.catalogue.read_text())
    live = []
    snapshots = []
    screening = []
    for index, item in enumerate(catalogue):
        site = item["site"]
        mode = item["network_mode"]
        archive = output / ("snapshot-archive-" + site)
        if item["url"] != SITES[site]:
            raise ValueError(f"Catalogue URL differs from the configured site: {site}")
        live.append(
            {k: item[k] for k in ["site", "url", "network_mode"]}
            | {"order_index": index}
        )
        row = {"site": site, "eligible": False}
        start = time.monotonic()
        options = SimpleNamespace(
            binary=args.binary.resolve(),
            site=site,
            mode="record",
            archive=archive,
            output=output / ("capture-" + site),
            seconds=args.seconds,
            cpus=args.cpus,
            network_mode=mode,
            sample_interval=0.1,
            max_rss_mib=4096,
            idle_seconds=0,
            close_seconds=0,
            pulse_batches=1,
            pulse_updates=2,
            close_page=False,
            observe_only=False,
            load_timeout=0,
        )
        print("capturing", site, flush=True)
        try:
            await asyncio.wait_for(one_run(options), 180)
            data = json.loads((archive / "archive.json").read_text())
            snapshot = json.loads((archive / "snapshot.json").read_text())
            obs = data["observation"]
            status = (data.get("main_response") or {}).get("status", 200)
            if (
                status >= 400
                or obs["nodes"] < 20
                or re.search(
                    r"just a moment|403 forbidden|404 not found|access denied|安全验证",
                    obs["title"],
                    re.IGNORECASE,
                )
            ):
                raise RuntimeError("HTTP error or challenge page")
            if snapshot["openShadows"]:
                raise RuntimeError("Shadow DOM is not captured by this serializer")
            options.mode = "snapshot"
            options.output = output / ("admission-" + site)
            options.seconds = 2
            options.load_timeout = 30
            options.pulse_updates = 12
            options.idle_seconds = 1
            options.close_page = True
            options.close_seconds = 1
            await asyncio.wait_for(one_run(options), 180)
            row.update(eligible=True, nodes=obs["nodes"], title=obs["title"])
            snapshots.append(
                dict(
                    live[-1],
                    archive=str(archive),
                    pulse_updates=12 if obs["nodes"] > 5000 else 48,
                    repetitions=3,
                )
            )
        except Exception as error:  # noqa: BLE001 - every excluded page remains explicit evidence
            row["reason"] = repr(error)
            print("excluded from snapshots", site, repr(error), flush=True)
        row["seconds"] = time.monotonic() - start
        screening.append(row)
        for name, value in [
            ("live-plan-v2.json", live),
            ("snapshot-plan-v2.json", snapshots),
            ("admission.json", screening),
        ]:
            (output / name).write_text(json.dumps(value, indent=2, ensure_ascii=False))
    print(
        "catalogue complete",
        len(live),
        "live candidates",
        len(snapshots),
        "faithful snapshots",
        flush=True,
    )


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ["binary", "catalogue", "output"]:
        parser.add_argument("--" + name, type=Path, required=True)
    parser.add_argument("--seconds", type=float, default=12)
    parser.add_argument("--cpus", default="3,4")
    asyncio.run(capture(parser.parse_args()))
