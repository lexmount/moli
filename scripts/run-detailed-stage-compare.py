#!/usr/bin/env python3
"""Run detailed CDP stage compare for ifeng and huxiu."""
from __future__ import annotations

import asyncio
import json
import time
from pathlib import Path
from typing import Any

from moli_benchmark.chrome_dcl import (
    CdpDclDumpTimeoutError,
    _recv_command_response,
    _recv_until_dcl_or_binary_main_resource,
    _wait_for_cdp,
)
from moli_benchmark.config import REPO_ROOT
from moli_benchmark.target_serve import start_target_serve, stop_target_serve


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
        frame_id = navigate_response.get("result", {}).get("frameId")
        if frame_id is not None:
            frame_id = str(frame_id)
        try:
            binary_body = await _recv_until_dcl_or_binary_main_resource(
                client,
                session_id=session_id,
                frame_id=frame_id,
                deadline=deadline,
                seen=seen,
            )
        except TimeoutError as error:
            raise CdpDclDumpTimeoutError("DCL", error) from error
        timings["navigate_to_dcl_ms"] = (time.perf_counter() - t0) * 1000
        if binary_body is not None:
            timings["outer_html_ms"] = 0.0
            timings["html_length"] = len(binary_body)
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
            serve = start_target_serve(target, binary, 30.0)
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


def main() -> None:
    moli_bin = REPO_ROOT / "target" / "release" / "moli"
    if not moli_bin.exists():
        print(f"Error: moli binary not found at {moli_bin}")
        print("Please run: cargo build --release")
        return

    urls = [
        ("ifeng", "https://www.ifeng.com/"),
        ("huxiu", "https://www.huxiu.com/"),
    ]

    all_results = []
    for name, url in urls:
        print(f"\nTesting {name}: {url}")
        results = run_single_test(
            target="moli-cdp",
            binary=moli_bin,
            url=url,
            runs=3,
        )
        all_results.extend(results)

    # Save results
    output_file = REPO_ROOT / "tmp" / f"detailed-stage-compare-{int(time.time())}.json"
    output_file.parent.mkdir(parents=True, exist_ok=True)
    with open(output_file, "w") as f:
        json.dump(all_results, f, indent=2)

    print(f"\nResults saved to: {output_file}")

    # Print summary
    print("\n=== Detailed Summary ===")
    for name, url in urls:
        print(f"\n{name}:")
        url_results = [r for r in all_results if r["url"] == url]
        for r in url_results:
            timings = r.get("timings", {})
            print(f"  Run {r['run']}:")
            print(f"    Total elapsed: {r['elapsed_ms']:.1f}ms")
            if "error" in r or "error" in timings:
                error = r["error"] if "error" in r else timings["error"]
                print(f"    Error: {error or '<no error details>'}")
                continue
            print(f"    create_target: {timings.get('create_target_ms', 'N/A'):.1f}ms")
            print(f"    attach_target: {timings.get('attach_target_ms', 'N/A'):.1f}ms")
            print(f"    enable_domains: {timings.get('enable_domains_ms', 'N/A'):.1f}ms")
            print(f"    navigate_ack: {timings.get('navigate_ack_ms', 'N/A'):.1f}ms")
            print(f"    navigate_to_dcl: {timings.get('navigate_to_dcl_ms', 'N/A'):.1f}ms")
            print(f"    outer_html: {timings.get('outer_html_ms', 'N/A'):.1f}ms")
            print(f"    html_length: {timings.get('html_length', 'N/A')} bytes")


if __name__ == "__main__":
    main()
