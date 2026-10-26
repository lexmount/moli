#!/usr/bin/env python3
"""Run three-way CDP stage compare for ifeng and huxiu."""
from __future__ import annotations

import asyncio
import argparse
import json
import time
from pathlib import Path
from statistics import median
from typing import Any

from moli_benchmark.chrome_dcl import (
    CdpDclDumpTimeoutError,
    DEFAULT_CHROME_DCL_USER_AGENT,
    _parse_navigation_result,
    _recv_command_response,
    _recv_until_dcl_or_binary_main_resource,
    _wait_for_cdp,
)
from moli_benchmark.config import REPO_ROOT
from moli_benchmark.target_serve import start_target_serve, stop_target_serve
from moli_benchmark.targets import collect_target_binaries


def _serve_extra_args(target: str) -> tuple[str, ...]:
    if target == "chrome-cdp":
        return (f"--user-agent={DEFAULT_CHROME_DCL_USER_AGENT}",)
    return ()


async def _dump_dcl_with_timing(
    endpoint: str,
    process: Any,
    url: str,
    timeout_seconds: float,
) -> dict[str, Any]:
    """Run CDP DCL test and capture timing for each stage."""
    deadline = time.perf_counter() + timeout_seconds
    timings: dict[str, Any] = {}

    try:
        client = await _wait_for_cdp(endpoint, process, min(5.0, max(0.1, timeout_seconds)))
    except TimeoutError as error:
        return {"error": f"startup timeout: {error}"}

    target_id: str | None = None
    try:
        # Target.createTarget
        t0 = time.perf_counter()
        create_id = await client.send("Target.createTarget", {"url": "about:blank"})
        create_response, _ = await _recv_command_response(
            client, create_id, deadline=deadline, stage="Target.createTarget",
        )
        target_id = str(create_response["result"]["targetId"])
        timings["create_target_ms"] = (time.perf_counter() - t0) * 1000

        # Target.attachToTarget
        t0 = time.perf_counter()
        attach_id = await client.send("Target.attachToTarget", {"targetId": target_id, "flatten": True})
        attach_response, _ = await _recv_command_response(
            client, attach_id, deadline=deadline, stage="Target.attachToTarget",
        )
        session_id = str(attach_response["result"]["sessionId"])
        timings["attach_target_ms"] = (time.perf_counter() - t0) * 1000

        # Enable domains
        t0 = time.perf_counter()
        for method in ("Page.enable", "Runtime.enable", "Network.enable"):
            message_id = await client.send(method, session_id=session_id)
            await _recv_command_response(client, message_id, deadline=deadline, stage=method)
        lifecycle_id = await client.send("Page.setLifecycleEventsEnabled", {"enabled": True}, session_id=session_id)
        await _recv_command_response(client, lifecycle_id, deadline=deadline, stage="Page.setLifecycleEventsEnabled")
        timings["enable_domains_ms"] = (time.perf_counter() - t0) * 1000

        # Page.navigate
        t0 = time.perf_counter()
        navigate_id = await client.send("Page.navigate", {"url": url}, session_id=session_id)
        navigate_response, seen = await _recv_command_response(
            client, navigate_id, deadline=deadline, stage="Page.navigate",
        )
        timings["navigate_ack_ms"] = (time.perf_counter() - t0) * 1000

        # Wait for DCL
        t0 = time.perf_counter()
        navigation = _parse_navigation_result(navigate_response, url)
        try:
            main_document = await _recv_until_dcl_or_binary_main_resource(
                client,
                session_id=session_id,
                frame_id=navigation.frame_id,
                expected_loader_id=navigation.loader_id,
                deadline=deadline,
                seen=seen,
                download_navigation=navigation.is_download,
            )
        except TimeoutError as error:
            raise CdpDclDumpTimeoutError("DCL", error) from error
        timings["navigate_to_dcl_ms"] = (time.perf_counter() - t0) * 1000
        if main_document.headers_only:
            timings["outer_html_ms"] = 0.0
            timings["html_length"] = 0
            return timings

        # Runtime.evaluate for outerHTML
        t0 = time.perf_counter()
        evaluate_id = await client.send(
            "Runtime.evaluate",
            {
                "expression": "document.documentElement ? document.documentElement.outerHTML : ''",
                "returnByValue": True,
            },
            session_id=session_id,
        )
        evaluate_response, _ = await _recv_command_response(
            client, evaluate_id, deadline=deadline, stage="outerHTML",
        )
        timings["outer_html_ms"] = (time.perf_counter() - t0) * 1000

        result = evaluate_response.get("result", {}).get("result", {})
        value = result.get("value", "")
        timings["html_length"] = len(value) if isinstance(value, str) else 0

    finally:
        if target_id is not None:
            try:
                close_id = await client.send("Target.closeTarget", {"targetId": target_id})
                await client.recv_until_id(close_id, timeout=1.0)
            except Exception:
                pass
        await client.websocket.close()

    return timings


def run_single_test(target: str, binary: Path, url: str, runs: int = 3) -> list[dict[str, Any]]:
    """Run CDP stage test for a single target and URL."""
    results = []
    for i in range(runs):
        print(f"  Run {i+1}/{runs}: {target} - {url}")
        started = time.perf_counter()
        serve = None
        try:
            serve = start_target_serve(target, binary, 30.0, _serve_extra_args(target))
            timings = asyncio.run(_dump_dcl_with_timing(serve.endpoint, serve.process, url, 30.0))
            stopped = stop_target_serve(serve)
            serve = None
            elapsed_ms = (time.perf_counter() - started) * 1000
            results.append({
                "target": target,
                "url": url,
                "run": i + 1,
                "elapsed_ms": elapsed_ms,
                "timings": timings,
                "resources": stopped.get("resources", {}),
            })
        except Exception as e:
            elapsed_ms = (time.perf_counter() - started) * 1000
            results.append({
                "target": target,
                "url": url,
                "run": i + 1,
                "elapsed_ms": elapsed_ms,
                "error": str(e),
            })
            if serve is not None:
                stop_target_serve(serve)
    return results


def _parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--runs",
        type=int,
        default=3,
        help="number of runs per target and URL",
    )
    args = parser.parse_args()
    if args.runs < 1:
        parser.error("--runs must be at least 1")
    return args


def main() -> None:
    args = _parse_args()
    moli_bin = REPO_ROOT / "target" / "release" / "moli"
    if not moli_bin.exists():
        print(f"Error: moli binary not found at {moli_bin}")
        print("Please run: cargo build --release")
        return

    target_matrix = collect_target_binaries(
        moli_override=str(moli_bin),
    )

    urls = [
        ("ifeng", "https://www.ifeng.com/"),
        ("huxiu", "https://www.huxiu.com/"),
    ]

    targets = []
    for target_name in ("moli", "lightpanda", "chrome"):
        target_info = target_matrix.get(target_name)
        if target_info and target_info.get("available"):
            targets.append((f"{target_name}-cdp", Path(target_info["path"])))
        else:
            print(f"Warning: {target_name} not available")

    all_results = []
    for name, url in urls:
        print(f"\nTesting {name}: {url}")
        for target_name, binary_path in targets:
            if binary_path is None:
                print(f"  Skipping {target_name}: binary not found")
                continue
            print(f"\n  Target: {target_name}")
            results = run_single_test(
                target=target_name,
                binary=binary_path,
                url=url,
                runs=args.runs,
            )
            all_results.extend(results)

    # Save results
    output_file = REPO_ROOT / "tmp" / f"three-way-compare-{int(time.time())}.json"
    output_file.parent.mkdir(parents=True, exist_ok=True)
    with open(output_file, "w") as f:
        json.dump(all_results, f, indent=2)

    print(f"\nResults saved to: {output_file}")

    # Print summary table
    print("\n=== Three-Way Comparison Summary ===")
    for name, url in urls:
        print(f"\n{name}:")
        print(
            f"{'Target':<20} {'navigate_ack':>12} {'ack_to_dcl':>12} "
            f"{'nav_to_dcl':>12} {'outer_html':>10} {'html_bytes':>10}"
        )
        print("-" * 82)
        for target_name, _ in targets:
            target_results = [r for r in all_results if r["url"] == url and r["target"] == target_name]
            if not target_results:
                continue
            complete_timings = [
                r["timings"]
                for r in target_results
                if "timings" in r
                and "navigate_ack_ms" in r["timings"]
                and "navigate_to_dcl_ms" in r["timings"]
                and "outer_html_ms" in r["timings"]
                and "html_length" in r["timings"]
            ]
            if complete_timings:
                ack_times = [t["navigate_ack_ms"] for t in complete_timings]
                ack_to_dcl_times = [t["navigate_to_dcl_ms"] for t in complete_timings]
                nav_to_dcl_times = [
                    t["navigate_ack_ms"] + t["navigate_to_dcl_ms"]
                    for t in complete_timings
                ]
                outer_times = [t["outer_html_ms"] for t in complete_timings]
                html_lengths = [t["html_length"] for t in complete_timings]
                print(
                    f"{target_name:<20} {median(ack_times):>10.1f}ms "
                    f"{median(ack_to_dcl_times):>10.1f}ms "
                    f"{median(nav_to_dcl_times):>10.1f}ms "
                    f"{median(outer_times):>8.1f}ms "
                    f"{median(html_lengths):>10.0f}"
                )


if __name__ == "__main__":
    main()
