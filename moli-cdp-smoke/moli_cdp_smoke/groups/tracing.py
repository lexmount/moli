from __future__ import annotations


import asyncio
import base64
import json
from collections import Counter, defaultdict
from dataclasses import dataclass
from typing import Any

from ..diagnostics import new_cdp_session, write_artifact
from . import SmokeState
from ..assertions import SmokeError, assert_equal, record, wait_until
from ..helpers import attach_cdp_event_collector
from ..raw_cdp import (
    RawCdpClient,
    connect_raw_cdp,
    connect_raw_cdp_websocket,
    discover_page_websocket_url,
)


async def run_tracing_group(state: SmokeState) -> None:
    await _verify_report_events_and_global_owner(state)
    await _verify_agent_browser_profiler_configuration(state)
    await _verify_return_as_stream(state)
    await _verify_owner_detach_cleanup(state)


async def _verify_report_events_and_global_owner(state: SmokeState) -> None:
    owner = state.cdp
    peer = await new_cdp_session(state.context, state.page)
    events = attach_cdp_event_collector(
        owner,
        ["Tracing.dataCollected", "Tracing.tracingComplete"],
    )
    try:
        categories = await owner.send("Tracing.getCategories")
        if not isinstance(categories.get("categories"), list) or not categories["categories"]:
            raise SmokeError(f"Tracing.getCategories returned no categories: {categories!r}")

        await owner.send(
            "Tracing.start",
            {
                "categories": "__metadata,disabled-by-default-devtools.timeline",
                "transferMode": "ReportEvents",
            },
        )
        await _expect_protocol_error(
            owner,
            "Tracing.start",
            {},
            "Tracing has already been started (possibly in another tab).",
        )
        await _expect_protocol_error(
            peer,
            "Tracing.start",
            {},
            "Tracing has already been started (possibly in another tab).",
        )
        await _expect_protocol_error(peer, "Tracing.end", {}, "Tracing is not started")
        await peer.send("Tracing.recordClockSyncMarker", {"syncId": "smoke-peer-marker"})

        await owner.send("Tracing.end")
        await wait_until(
            lambda: any(event["method"] == "Tracing.tracingComplete" for event in events),
            "Tracing.tracingComplete in ReportEvents mode",
        )

        methods = [event["method"] for event in events]
        if not methods or methods[-1] != "Tracing.tracingComplete":
            raise SmokeError(f"trace events must finish with tracingComplete: {methods!r}")
        if "Tracing.dataCollected" not in methods:
            raise SmokeError(f"ReportEvents trace did not emit dataCollected: {methods!r}")
        trace_events = [
            trace_event
            for event in events
            if event["method"] == "Tracing.dataCollected"
            for trace_event in event["params"].get("value", [])
        ]
        write_artifact("tracing-report-events.json", {"traceEvents": trace_events})
        _assert_trace_event_array(trace_events, "ReportEvents")
        marker = next(
            (
                event
                for event in trace_events
                if event.get("name") == "clock_sync"
                and event.get("args", {}).get("sync_id") == "smoke-peer-marker"
            ),
            None,
        )
        if marker is None:
            raise SmokeError("peer clock marker was not recorded in browser-global trace")
        complete = events[-1]["params"]
        assert_equal(complete.get("dataLossOccurred"), False, "ReportEvents data loss")
        if "stream" in complete:
            raise SmokeError(f"ReportEvents tracingComplete must not expose a stream: {complete!r}")
        state.record(
            "tracing_report_events_global_owner",
            {"traceEventCount": len(trace_events), "categoryCount": len(categories["categories"])},
        )
    finally:
        events.close()
        await peer.detach()


async def run_raw_tracing_group(
    endpoint: str,
    _fixture: str,
    results: list[dict[str, Any]],
) -> None:
    client = await connect_raw_cdp(endpoint)
    peer: RawCdpClient | None = None
    try:
        peer = await connect_raw_cdp_websocket(await discover_page_websocket_url(endpoint))
        start_id = await client.send(
            "Tracing.start",
            {
                "categories": "__metadata",
                "transferMode": "ReportEvents",
            },
        )
        await client.recv_until_id(start_id, timeout=5)
        peer_start_id = await peer.send(
            "Tracing.start",
            {
                "categories": "devtools.timeline",
                "transferMode": "ReportEvents",
            },
        )
        peer_start = await _recv_raw_response(peer, peer_start_id)
        assert_equal(
            peer_start.get("error"),
            {
                "code": -32000,
                "message": "Tracing has already been started (possibly in another tab).",
            },
            "Tracing browser-global ownership across physical frontends",
        )
        marker_id = await client.send(
            "Tracing.recordClockSyncMarker",
            {"syncId": "raw-wire-order"},
        )
        await client.recv_until_id(marker_id, timeout=5)

        end_id = await client.send("Tracing.end")
        sequence: list[str] = []
        deadline = asyncio.get_running_loop().time() + 5.0
        while "complete" not in sequence:
            remaining = deadline - asyncio.get_running_loop().time()
            if remaining <= 0:
                raise SmokeError(f"timed out reading raw Tracing.end output: {sequence!r}")
            message = await asyncio.wait_for(client.recv(), timeout=remaining)
            if message.get("id") == end_id:
                if "error" in message:
                    raise SmokeError(f"raw Tracing.end failed: {message['error']!r}")
                sequence.append("response")
            elif message.get("id") == start_id:
                raise SmokeError(
                    "synchronous Tracing.start received a second response after "
                    f"Tracing.end: {message!r}"
                )
            elif message.get("method") == "Tracing.dataCollected":
                sequence.append("data")
            elif message.get("method") == "Tracing.tracingComplete":
                sequence.append("complete")

        if not sequence or sequence[0] != "response":
            raise SmokeError(f"Tracing.end response must precede trace output: {sequence!r}")
        if "data" not in sequence or sequence.index("data") > sequence.index("complete"):
            raise SmokeError(f"trace data must precede tracingComplete: {sequence!r}")
        record(
            results,
            "tracing_raw_end_wire_order",
            {"sequence": sequence, "physicalFrontendPeerRejected": True},
        )
        await _verify_stop_before_start_ack_wire_order(client, results)
    finally:
        if peer is not None:
            await peer.websocket.close()
        await client.websocket.close()


async def _verify_stop_before_start_ack_wire_order(
    client: RawCdpClient,
    results: list[dict[str, Any]],
) -> None:
    start_id = await client.send(
        "Tracing.start",
        {
            "traceConfig": {
                "includedCategories": ["disabled-by-default-v8.cpu_profiler"]
            },
            "transferMode": "ReportEvents",
        },
    )
    end_id = await client.send("Tracing.end")
    responses: dict[int, dict[str, Any]] = {}
    sequence: list[str] = []
    saw_complete = False
    deadline = asyncio.get_running_loop().time() + 5.0
    while len(responses) < 2 or not saw_complete:
        remaining = deadline - asyncio.get_running_loop().time()
        if remaining <= 0:
            raise SmokeError(
                "timed out reading stop-before-start-ack output: "
                f"responses={responses!r}, sequence={sequence!r}"
            )
        message = await asyncio.wait_for(client.recv(), timeout=remaining)
        message_id = message.get("id")
        if message_id == end_id:
            responses[end_id] = message
            sequence.append("end-response")
        elif message_id == start_id:
            responses[start_id] = message
            sequence.append("start-error" if "error" in message else "start-response")
        elif message.get("method") == "Tracing.dataCollected":
            sequence.append("data")
        elif message.get("method") == "Tracing.tracingComplete":
            sequence.append("complete")
            saw_complete = True

    assert_equal(responses[end_id].get("result"), {}, "racing Tracing.end response")
    assert_equal(
        responses[start_id].get("error"),
        {
            "code": -32000,
            "message": "Tracing was stopped before start has been completed.",
        },
        "stopped-before-ack Tracing.start error",
    )
    if sequence[:2] != ["end-response", "start-error"]:
        raise SmokeError(
            "Chromium sends the racing Tracing.end response before the interrupted "
            f"Tracing.start error: {sequence!r}"
        )
    if "data" not in sequence or sequence.index("data") < 2:
        raise SmokeError(f"interrupted trace data must follow both responses: {sequence!r}")
    if sequence[-1] != "complete":
        raise SmokeError(f"interrupted trace must finish with tracingComplete: {sequence!r}")
    record(
        results,
        "tracing_raw_stop_before_start_ack_wire_order",
        {"sequence": sequence, "error": responses[start_id]["error"]},
    )


async def _recv_raw_response(client: RawCdpClient, message_id: int) -> dict[str, Any]:
    deadline = asyncio.get_running_loop().time() + 5.0
    while True:
        remaining = deadline - asyncio.get_running_loop().time()
        if remaining <= 0:
            raise SmokeError(f"timed out waiting for raw CDP response id={message_id}")
        message = await asyncio.wait_for(client.recv(), timeout=remaining)
        if message.get("id") == message_id:
            return message


async def _verify_agent_browser_profiler_configuration(state: SmokeState) -> None:
    expected_functions = {
        "moliCpuTraceBeforeNavigation",
        "moliCpuTraceAfterNavigation",
        "moliCpuTraceWorkerHotFunction",
        "moliCpuTraceSharedWorkerHotFunction",
        "moliCpuTraceClosedPageHotFunction",
    }
    with attach_cdp_event_collector(
        state.cdp,
        ["Tracing.dataCollected", "Tracing.tracingComplete"],
    ) as events:
        await state.cdp.send(
            "Tracing.start",
            {
                "traceConfig": {
                    "includedCategories": [
                        "devtools.timeline",
                        "disabled-by-default-devtools.timeline",
                        "disabled-by-default-v8.cpu_profiler",
                        "v8.execute",
                    ],
                    "enableSampling": True,
                },
                "transferMode": "ReportEvents",
            },
        )
        try:
            await state.cdp.send(
                "Runtime.evaluate",
                {
                    "expression": _hot_function_expression("moliCpuTraceBeforeNavigation"),
                    "returnByValue": True,
                },
            )
            await state.page.goto(
                f"{state.fixture}/plain?trace=after-navigation",
                wait_until="load",
                timeout=10_000,
            )
            await state.page.evaluate(_hot_function_expression("moliCpuTraceAfterNavigation"))
            await state.page.evaluate(
                """
                async () => {
                  const source = `
                    self.onmessage = () => {
                      function moliCpuTraceWorkerHotFunction() {
                        const deadline = performance.now() + 120;
                        let value = 1;
                        while (performance.now() < deadline) {
                          value = Math.imul(value + 3, 1103515245) | 0;
                        }
                        return value;
                      }
                      self.postMessage(moliCpuTraceWorkerHotFunction());
                    };
                  `;
                  const url = URL.createObjectURL(new Blob([source], {type: 'text/javascript'}));
                  const worker = new Worker(url);
                  try {
                    await new Promise((resolve, reject) => {
                      const timer = setTimeout(
                        () => reject(new Error('dedicated worker CPU trace timed out')),
                        5_000
                      );
                      worker.onmessage = event => {
                        clearTimeout(timer);
                        resolve(event);
                      };
                      worker.onerror = error => {
                        clearTimeout(timer);
                        reject(error);
                      };
                      worker.postMessage('run');
                    });
                  } finally {
                    worker.terminate();
                    URL.revokeObjectURL(url);
                  }
                }
                """
            )
            await state.page.evaluate(
                """
                async () => {
                  const worker = new SharedWorker(
                    '/shared-worker.js?cpu-trace',
                    'moli-cpu-trace-shared-worker'
                  );
                  try {
                    await new Promise((resolve, reject) => {
                      const timer = setTimeout(
                        () => reject(new Error('shared worker CPU trace timed out')),
                        5_000
                      );
                      worker.port.onmessage = event => {
                        if (event.data && event.data.kind === 'cpu-trace-result') {
                          clearTimeout(timer);
                          resolve();
                        }
                      };
                      worker.port.onmessageerror = error => {
                        clearTimeout(timer);
                        reject(error);
                      };
                      worker.port.start();
                      worker.port.postMessage({kind: 'cpu-trace'});
                    });
                  } finally {
                    worker.port.close();
                  }
                }
                """
            )
            closed_page = await state.context.new_page()
            try:
                await closed_page.goto(
                    f"{state.fixture}/plain?trace=closed-page",
                    wait_until="load",
                    timeout=10_000,
                )
                await closed_page.evaluate(
                    _hot_function_expression("moliCpuTraceClosedPageHotFunction")
                )
            finally:
                await closed_page.close()
        finally:
            try:
                await state.cdp.send("Tracing.end")
                await wait_until(
                    lambda: any(event["method"] == "Tracing.tracingComplete" for event in events),
                    "Tracing.tracingComplete for agent-browser profiler configuration",
                )
            finally:
                trace_events = [
                    trace_event
                    for event in events
                    if event["method"] == "Tracing.dataCollected"
                    for trace_event in event["params"].get("value", [])
                ]
                complete = next((
                    event["params"]
                    for event in reversed(events)
                    if event["method"] == "Tracing.tracingComplete"
                ), None)
                artifact = write_artifact("tracing-cpu.json", {
                    "traceEvents": trace_events, "expectedFunctions": sorted(expected_functions),
                    "tracingComplete": complete,
                })
        _assert_trace_event_array(trace_events, "agent-browser profiler configuration")
        profile = _assert_cpu_profile_events(trace_events, expected_functions)
        assert_equal(complete.get("dataLossOccurred"), False, "CPU profile data loss")
        state.record(
            "tracing_agent_browser_profiler_configuration",
            {
                "traceEventCount": len(trace_events),
                "profileCount": profile.profile_count,
                "sampleCount": profile.sample_count,
                "functionSamples": profile.function_samples,
                "traceArtifact": artifact,
            },
        )


async def _verify_return_as_stream(state: SmokeState) -> None:
    with attach_cdp_event_collector(state.cdp, ["Tracing.tracingComplete"]) as complete_events:
        start_index = len(complete_events)
        await state.cdp.send(
            "Tracing.start",
            {
                "transferMode": "ReturnAsStream",
                "streamFormat": "json",
                "streamCompression": "none",
                "traceConfig": {
                    "recordMode": "recordContinuously",
                    "excludedCategories": ["*"],
                    "includedCategories": [
                        "devtools.timeline",
                        "v8.execute",
                        "disabled-by-default-devtools.timeline",
                        "disabled-by-default-devtools.timeline.frame",
                        "disabled-by-default-devtools.timeline.stack",
                        "disabled-by-default-v8.cpu_profiler",
                        "blink.user_timing",
                    ],
                },
            },
        )
        await state.cdp.send(
            "Runtime.evaluate",
            {
                "expression": "globalThis.__moliTracingSmoke = (globalThis.__moliTracingSmoke || 0) + 1",
                "returnByValue": True,
            },
        )
        await state.cdp.send("Tracing.end")
        await wait_until(
            lambda: len(complete_events) > start_index,
            "Tracing.tracingComplete in ReturnAsStream mode",
        )
        complete = complete_events[-1]["params"]
        handle = complete.get("stream")
        if not isinstance(handle, str) or not handle:
            raise SmokeError(f"ReturnAsStream tracingComplete has no stream: {complete!r}")
        assert_equal(complete.get("dataLossOccurred"), False, "stream trace data loss")
        assert_equal(complete.get("traceFormat"), "json", "stream trace format")
        assert_equal(complete.get("streamCompression"), "none", "stream trace compression")

        encoded = bytearray()
        while True:
            part = await state.cdp.send("IO.read", {"handle": handle, "size": 64 * 1024})
            data = part.get("data", "")
            if part.get("base64Encoded"):
                encoded.extend(base64.b64decode(data))
            else:
                encoded.extend(data.encode("utf-8"))
            if part.get("eof"):
                break
        await state.cdp.send("IO.close", {"handle": handle})
        write_artifact("tracing-stream-raw.json", {"base64": base64.b64encode(encoded).decode("ascii")})
        try:
            trace = json.loads(encoded)
        except (UnicodeDecodeError, json.JSONDecodeError) as error:
            raise SmokeError(f"Tracing stream is not valid UTF-8 JSON: {error}") from error
        write_artifact("tracing-stream.json", trace)
        trace_events = trace.get("traceEvents")
        if not isinstance(trace_events, list):
            raise SmokeError(f"Tracing stream has no traceEvents array: {trace!r}")
        _assert_trace_event_array(trace_events, "ReturnAsStream")
        if not isinstance(trace.get("metadata"), dict):
            raise SmokeError("Tracing JSON stream must include its metadata object")
        state.record(
            "tracing_return_as_stream",
            {"traceEventCount": len(trace_events), "streamBytes": len(encoded)},
        )


async def _verify_owner_detach_cleanup(state: SmokeState) -> None:
    owner = await new_cdp_session(state.context, state.page)
    await owner.send(
        "Tracing.start",
        {
            "traceConfig": {
                "includedCategories": [
                    "__metadata",
                    "disabled-by-default-v8.cpu_profiler",
                ]
            }
        },
    )
    await state.page.evaluate(_hot_function_expression("moliCpuTraceDetachedOwner"))
    await owner.detach()

    replacement = await new_cdp_session(state.context, state.page)
    complete: list[dict[str, Any]] = []
    replacement.on("Tracing.tracingComplete", lambda params: complete.append(params))
    try:
        await replacement.send(
            "Tracing.start",
            {
                "traceConfig": {
                    "includedCategories": [
                        "__metadata",
                        "disabled-by-default-v8.cpu_profiler",
                    ]
                }
            },
        )
        await state.page.evaluate(_hot_function_expression("moliCpuTraceReplacementOwner"))
        await replacement.send("Tracing.end")
        await wait_until(lambda: bool(complete), "trace restart after owner detach")
        state.record("tracing_owner_detach_cleanup")
    finally:
        await replacement.detach()


async def _expect_protocol_error(
    session: Any,
    method: str,
    params: dict[str, Any],
    expected_message: str,
) -> None:
    try:
        await session.send(method, params)
    except Exception as error:
        if expected_message not in str(error):
            raise SmokeError(
                f"{method} expected error containing {expected_message!r}, got {error!s}"
            ) from error
        return
    raise SmokeError(f"{method} unexpectedly succeeded; expected {expected_message!r}")


def _assert_trace_event_array(events: list[Any], label: str) -> None:
    if not events:
        raise SmokeError(f"{label} trace must contain real events")
    for index, event in enumerate(events):
        if not isinstance(event, dict):
            raise SmokeError(f"{label} trace event {index} is not an object: {event!r}")
        missing = [key for key in ("cat", "name", "ph", "pid", "tid", "ts") if key not in event]
        if missing:
            raise SmokeError(f"{label} trace event {index} is missing {missing!r}: {event!r}")


def _hot_function_expression(function_name: str) -> str:
    return f"""
        (() => {{
          function {function_name}() {{
            const deadline = performance.now() + 120;
            let value = 1;
            while (performance.now() < deadline) {{
              value = Math.imul(value + 3, 1103515245) | 0;
            }}
            return value;
          }}
          return {function_name}();
        }})()
    """


@dataclass(frozen=True)
class CpuTraceSummary:
    profile_count: int
    sample_count: int
    function_samples: dict[str, int]


def _assert_cpu_profile_events(
    events: list[dict[str, Any]], function_names: set[str]
) -> CpuTraceSummary:
    profiles = [event for event in events if event.get("name") == "Profile"]
    chunks = [event for event in events if event.get("name") == "ProfileChunk"]
    if not profiles or not chunks:
        raise SmokeError(
            f"CPU tracing must emit Profile and ProfileChunk events: "
            f"profiles={len(profiles)}, chunks={len(chunks)}"
        )

    # V8 profile IDs are process-local. Different renderer processes routinely
    # use the same ID, and ProfileChunk can originate on another thread.
    profile_ids = {(event.get("pid"), event.get("id")) for event in profiles}
    if (any(identity[1] is None for identity in profile_ids)
            or any((event.get("pid"), event.get("id")) not in profile_ids for event in chunks)):
        raise SmokeError("CPU ProfileChunk ids must identify a preceding Profile event")
    for profile in profiles:
        data = profile.get("args", {}).get("data", {})
        if data.get("source") != "Internal" or "startTime" not in data:
            raise SmokeError(f"CPU Profile has an invalid source/startTime: {profile!r}")

    sample_count = 0
    observed_function_names: set[str] = set()
    nodes_by_profile: dict[Any, dict[int, Any]] = defaultdict(dict)
    samples_by_profile: dict[Any, list[int]] = defaultdict(list)
    for chunk in chunks:
        data = chunk.get("args", {}).get("data", {})
        if data.get("source") != "Internal":
            raise SmokeError(f"CPU ProfileChunk has an invalid source: {chunk!r}")
        cpu_profile = data.get("cpuProfile", {})
        nodes = cpu_profile.get("nodes", [])
        samples = cpu_profile.get("samples", [])
        time_deltas = data.get("timeDeltas", [])
        if not isinstance(nodes, list) or not isinstance(samples, list):
            raise SmokeError(f"CPU ProfileChunk nodes/samples must be arrays: {chunk!r}")
        if samples:
            if not isinstance(time_deltas, list) or len(samples) != len(time_deltas):
                raise SmokeError(
                    "CPU ProfileChunk samples and timeDeltas must have equal lengths"
                )
            sample_count += len(samples)
        identity = chunk.get("pid"), chunk.get("id")
        nodes_by_profile[identity].update((node.get("id"), node) for node in nodes if isinstance(node, dict))
        samples_by_profile[identity].extend(samples)
        observed_function_names.update(
            node.get("callFrame", {}).get("functionName")
            for node in nodes
            if isinstance(node, dict)
        )

    if sample_count == 0:
        raise SmokeError("CPU tracing produced no samples")
    missing_functions = function_names - observed_function_names
    if missing_functions:
        raise SmokeError(
            f"CPU tracing did not report named hot functions {sorted(missing_functions)!r}"
        )

    function_samples = dict.fromkeys(sorted(function_names), 0)
    for identity, samples in samples_by_profile.items():
        nodes = nodes_by_profile[identity]
        parents = {node_id: node.get("parent") for node_id, node in nodes.items()}
        for node_id, node in nodes.items():
            for child in node.get("children", []):
                parents[child] = node_id
        for node_id, count in Counter(samples).items():
            visited: set[int] = set()
            sampled_functions: set[str] = set()
            while node_id is not None:
                if node_id in visited or node_id not in nodes:
                    raise SmokeError(f"CPU tracing has an invalid sample stack profile={identity} node={node_id}")
                visited.add(node_id)
                name = nodes[node_id].get("callFrame", {}).get("functionName")
                if name in function_samples:
                    sampled_functions.add(name)
                node_id = parents.get(node_id)
            for name in sampled_functions:
                function_samples[name] += count
    missing_samples = [name for name, count in function_samples.items() if count == 0]
    if missing_samples:
        raise SmokeError(f"CPU tracing did not sample named hot functions {missing_samples!r}")
    return CpuTraceSummary(len(profiles), sample_count, function_samples)
