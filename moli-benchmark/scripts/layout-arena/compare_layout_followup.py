"""Compare real-page relayout under viewport, wrapping and box-lifetime changes."""

from __future__ import annotations

import argparse
import asyncio
import datetime
import hashlib
import json
import os
import platform
import statistics
import time
from pathlib import Path
from types import SimpleNamespace

from compare_webpages import native_metrics
from web_bench import one_run, proc_resources

VARIANTS = ("baseline", "slotmap_before", "dense_before", "slotmap", "dense")
SCENARIOS = ("viewport", "container_width", "text", "display", "detach")
VIEWPORTS = ((1024, 768), (768, 1024), (390, 844), (1280, 720))
JS = (Path(__file__).parent / "layout_scenarios.js").read_text()


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def verify_bodies(archive):
    entries = json.loads((archive / "archive.json").read_text())["entries"]
    hashes = {entry["body_sha256"] for entry in entries if entry.get("body_sha256")}
    for expected in hashes:
        if digest(archive / "bodies" / expected) != expected:
            raise RuntimeError(f"Archived body hash mismatch in {archive}")
    return len(hashes)


def signature(value):
    return hashlib.sha256(
        json.dumps(value, sort_keys=True, separators=(",", ":")).encode()
    ).hexdigest()


async def viewport(client, width=1280, height=720):
    await client.command(
        "Emulation.setDeviceMetricsOverride",
        {
            "width": width,
            "height": height,
            "deviceScaleFactor": 1,
            "mobile": False,
        },
    )


async def scenario_work(client, proc, sampler, steps):
    setup = await client.evaluate(JS)
    results = {"setup": setup, "scenarios": []}

    async def reset():
        await viewport(client)
        await client.evaluate("window.__arenaLayout.restore();true")
        await client.evaluate("document.body.getBoundingClientRect().height")

    for scenario in SCENARIOS:
        sampler.phase = "warmup_" + scenario
        for i in range(4):
            if scenario == "viewport":
                await viewport(client, *VIEWPORTS[i % 4])
            await client.evaluate(
                f"window.__arenaLayout.step({json.dumps(scenario)},{i})"
            )
        await reset()
        await asyncio.sleep(0.1)
        await client.evaluate("document.fonts.ready.then(()=>true)")
        samples = []
        initial = await client.evaluate("window.__arenaLayout.observe()")
        sampler.phase = scenario
        group_before = proc_resources(proc.pid)
        for i in range(steps):
            before = proc_resources(proc.pid)
            if scenario == "viewport":
                await viewport(client, *VIEWPORTS[i % 4])
            timed = await client.evaluate(
                f"window.__arenaLayout.step({json.dumps(scenario)},{i})"
            )
            after = proc_resources(proc.pid)
            # Full sampled geometry is deliberately outside the timed window.
            geometry = await client.evaluate("window.__arenaLayout.observe()")
            samples.append(
                {
                    "before": before,
                    "after": after,
                    "timed": timed,
                    "signature": signature(geometry),
                    "geometry": geometry,
                }
            )
        group_after = proc_resources(proc.pid)
        heap_after = await client.command("Runtime.getHeapUsage")
        await reset()
        final = await client.evaluate("window.__arenaLayout.observe()")
        if initial != final:
            raise RuntimeError(f"{scenario}: did not restore original DOM/geometry")
        if len({s["signature"] for s in samples}) < 2:
            raise RuntimeError(
                f"{scenario}: workload did not change sampled content/layout"
            )
        results["scenarios"].append(
            {
                "name": scenario,
                "steps": samples,
                "before": group_before,
                "after": group_after,
                "restored_signature": signature(final),
                "heap_after": heap_after,
            }
        )
        print("scenario", scenario, "complete", flush=True)
    await reset()
    return results


def extract(folder, result):
    output = []
    for group in result["layout_scenarios"]["scenarios"]:
        cpu = sum(
            (s["after"]["cpu_ns"] - s["before"]["cpu_ns"]) / 1_000_000
            for s in group["steps"]
        )
        wall = sum(
            (s["after"]["monotonic"] - s["before"]["monotonic"]) * 1000
            for s in group["steps"]
        )
        native = native_metrics(
            folder, {"before_pulse": group["before"], "after_pulse": group["after"]}
        )
        output.append(
            {
                "scenario": group["name"],
                "steps": len(group["steps"]),
                "cpu_ms_per_step": cpu / len(group["steps"]),
                "wall_ms_per_step": wall / len(group["steps"]),
                "js_ms_per_step": statistics.mean(
                    s["timed"]["ms"] for s in group["steps"]
                ),
                "average_cpu_percent": cpu / wall * 100,
                "rss_mib": group["after"]["memory_kib"]["Rss"] / 1024,
                "js_heap_used_mib": group["heap_after"]["usedSize"] / (1024 * 1024),
                "js_heap_total_mib": group["heap_after"]["totalSize"] / (1024 * 1024),
                "pss_mib": group["after"]["memory_kib"]["Pss"] / 1024,
                "rss_before_mib": group["before"]["memory_kib"]["Rss"] / 1024,
                "pss_before_mib": group["before"]["memory_kib"]["Pss"] / 1024,
                "kernel_peak_rss_mib": group["after"]["peak_rss_kib"] / 1024,
                "native": native,
                "signatures": [s["signature"] for s in group["steps"]],
                "restored_signature": group["restored_signature"],
            }
        )
    return output


async def run(args):
    args.output.mkdir(parents=True, exist_ok=False)
    plan = json.loads(args.plan.read_text())
    if args.sites:
        plan = [item for item in plan if item["site"] in args.sites.split(",")]
    variants_requested = tuple(args.variants.split(","))
    if any(v not in VARIANTS for v in variants_requested):
        raise ValueError("Unknown variant")
    binaries = {v: args.binaries / (v + "-moli") for v in variants_requested}
    verified_bodies = {
        item["site"]: verify_bodies(Path(item["archive"])) for item in plan
    }
    manifest = {
        "started_at": datetime.datetime.now(datetime.UTC).isoformat(),
        "plan": plan,
        "repetitions": args.repetitions,
        "cpus": args.cpus,
        "platform": platform.platform(),
        "clock_ticks": os.sysconf("SC_CLK_TCK"),
        "binary_sha256": {v: digest(p) for v, p in binaries.items()},
        "control_sha256": {
            p: digest(Path(__file__).parent / p)
            for p in [
                "web_bench.py",
                "snapshot_capture.js",
                "compare_webpages.py",
                "compare_layout_followup.py",
                "layout_scenarios.js",
            ]
        },
        "inputs": {
            p["site"]: {
                "verified_bodies": verified_bodies[p["site"]],
                **{
                    n: digest(Path(p["archive"]) / n)
                    for n in ["archive.json", "snapshot.json"]
                },
            }
            for p in plan
        },
        "steps_normal": args.steps,
        "steps_large": args.large_steps,
        "viewport_sequence": VIEWPORTS,
        "scenarios": SCENARIOS,
        "method": "Frozen real DOM/assets; application scripts inert. Fresh processes; rotating variants. Warm each scenario once; exact cross-variant per-step geometry validation. Timed mutations plus forced layout; geometry inspection outside timing. CPU is the Linux process CPU clock in nanoseconds (all threads), percent uses 100%=one core. No GC/trim. Initial original-capture fidelity and final restoration required.",
    }
    (args.output / "manifest.json").write_text(json.dumps(manifest, indent=2))
    rows, references = [], {}
    for repeat in range(args.repetitions):
        for index, item in enumerate(plan):
            rotation = (repeat + item.get("order_index", index)) % len(
                variants_requested
            )
            variants = variants_requested[rotation:] + variants_requested[:rotation]
            for variant in variants:
                name = f"{repeat + 1:02d}-{item['site']}-{variant}"
                folder = args.output / name
                row = {
                    "run": name,
                    "site": item["site"],
                    "variant": variant,
                    "repeat": repeat + 1,
                    "status": "failed",
                }
                print("running", name, flush=True)
                started = time.monotonic()
                steps = args.large_steps if item["nodes"] > 5000 else args.steps

                async def runner(client, proc, sampler, count=steps):
                    return await scenario_work(client, proc, sampler, count)

                options = SimpleNamespace(
                    binary=binaries[variant],
                    site=item["site"],
                    mode="snapshot",
                    archive=Path(item["archive"]),
                    output=folder,
                    seconds=2,
                    cpus=args.cpus,
                    network_mode=item["network_mode"],
                    sample_interval=0.1,
                    max_rss_mib=4096,
                    idle_seconds=1,
                    close_seconds=1,
                    close_page=True,
                    observe_only=False,
                    load_timeout=30,
                    scenario_runner=runner,
                )
                try:
                    await asyncio.wait_for(one_run(options), 240)
                    result = json.loads((folder / "result.json").read_text())
                    if result["binary_sha256"] != manifest["binary_sha256"][variant]:
                        raise RuntimeError("Executable changed")
                    for field, filename in [
                        ("archive_sha256", "archive.json"),
                        ("snapshot_sha256", "snapshot.json"),
                    ]:
                        if result[field] != manifest["inputs"][item["site"]][filename]:
                            raise RuntimeError("Input changed")
                    metrics = extract(folder, result)
                    fingerprint = {
                        "setup": result["layout_scenarios"]["setup"],
                        "initial": {
                            k: result["observation"][k]
                            for k in ["title", "nodes", "text", "boxes"]
                        },
                        "scenarios": [
                            {
                                k: m[k]
                                for k in [
                                    "scenario",
                                    "signatures",
                                    "restored_signature",
                                ]
                            }
                            for m in metrics
                        ],
                    }
                    site = item["site"]
                    if site in references and fingerprint != references[site]:
                        raise RuntimeError(
                            "Per-step DOM or geometry differs across variants/repetitions"
                        )
                    references[site] = fingerprint
                    row.update(
                        status="ok",
                        metrics=metrics,
                        setup={
                            k: v
                            for k, v in result["layout_scenarios"]["setup"].items()
                            if k != "initial"
                        },
                        idle_rss_mib=result["after_idle"]["memory_kib"]["Rss"] / 1024,
                        closed_rss_mib=result["after_close"]["memory_kib"]["Rss"]
                        / 1024,
                    )
                except Exception as error:  # noqa: BLE001 - retain each failed trial, no silent retries
                    row["error"] = repr(error)
                    folder.mkdir(exist_ok=True)
                    (folder / "run-error.json").write_text(json.dumps(row, indent=2))
                    print("failed", name, repr(error), flush=True)
                row["seconds"] = time.monotonic() - started
                rows.append(row)
                (args.output / "runs.json").write_text(json.dumps(rows, indent=2))
    manifest.update(
        finished_at=datetime.datetime.now(datetime.UTC).isoformat(),
        successes=sum(r["status"] == "ok" for r in rows),
        failures=sum(r["status"] != "ok" for r in rows),
    )
    for item in plan:
        verify_bodies(Path(item["archive"]))
    (args.output / "manifest.json").write_text(json.dumps(manifest, indent=2))
    print(
        "complete",
        manifest["successes"],
        "successful",
        manifest["failures"],
        "failed",
        flush=True,
    )


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ["plan", "binaries", "output"]:
        parser.add_argument("--" + name, type=Path, required=True)
    parser.add_argument("--sites")
    parser.add_argument(
        "--variants", default="baseline,slotmap_before,dense_before,slotmap,dense"
    )
    parser.add_argument("--repetitions", type=int, default=3)
    parser.add_argument("--steps", type=int, default=8)
    parser.add_argument("--large-steps", type=int, default=8)
    parser.add_argument("--cpus", default="3,4")
    asyncio.run(run(parser.parse_args()))
