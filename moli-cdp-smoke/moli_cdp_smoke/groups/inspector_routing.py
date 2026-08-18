from __future__ import annotations

import asyncio
import json
import os
import urllib.request
from dataclasses import dataclass
from typing import Any, Awaitable, Callable

from ..assertions import SmokeError, assert_equal, record_contract
from ..raw_cdp import RawCdpClient, connect_raw_cdp


@dataclass(frozen=True)
class InspectorRoutingPage:
    browser_context_id: str
    target_id: str
    primary_session_id: str
    auxiliary_session_id: str


async def run_inspector_routing_group(
    endpoint: str,
    fixture: str,
    results: list[dict[str, Any]],
) -> None:
    page_scenarios: list[
        tuple[
            str,
            Callable[
                [RawCdpClient, InspectorRoutingPage],
                Awaitable[None],
            ],
        ]
    ] = [
        (
            "raw_cdp_active_javascript_main_io_lane_matrix",
            lambda client, page: _active_javascript_lane_matrix(
                client, page, fixture, results
            ),
        ),
        (
            "raw_cdp_dedicated_worker_active_javascript_interrupt",
            lambda client, page: _worker_active_javascript_interrupt(
                client, page, fixture, results, target_type="worker"
            ),
        ),
        (
            "raw_cdp_shared_worker_active_javascript_interrupt",
            lambda client, page: _worker_active_javascript_interrupt(
                client, page, fixture, results, target_type="shared_worker"
            ),
        ),
        (
            "raw_cdp_debugger_io_catalog_during_active_javascript",
            lambda client, page: _debugger_io_catalog_during_active_javascript(
                client, page, fixture, results
            ),
        ),
        (
            "raw_cdp_performance_io_during_active_javascript",
            lambda client, page: _performance_io_during_active_javascript(
                client, page, fixture, results
            ),
        ),
        (
            "raw_cdp_script_execution_disabled_io_during_active_javascript",
            lambda client, page: _script_execution_disabled_io_during_active_javascript(
                client, page, fixture, results
            ),
        ),
        (
            "raw_cdp_nested_v8_main_receiver_matrix",
            lambda client, page: _nested_v8_main_receiver_matrix(
                client, page, results
            ),
        ),
        (
            "raw_cdp_nested_non_v8_main_receiver_matrix",
            lambda client, page: _nested_non_v8_main_receiver_matrix(
                client, page, results
            ),
        ),
        (
            "raw_cdp_instrumentation_pause_io_only_receiver",
            lambda client, page: _instrumentation_pause_io_only_receiver(
                client, page, results
            ),
        ),
        (
            "raw_cdp_navigation_replacement_during_active_javascript",
            lambda client, page: _navigation_replacement_during_active_javascript(
                client, page, fixture, results
            ),
        ),
        (
            "raw_cdp_session_detach_with_interrupts_in_flight",
            lambda client, page: _session_detach_with_interrupts_in_flight(
                client, page, fixture, results
            ),
        ),
        (
            "raw_cdp_context_dispose_with_interrupts_in_flight",
            lambda client, page: _context_dispose_with_interrupts_in_flight(
                client, page, fixture, results
            ),
        ),
    ]
    crash_scenario_name = "raw_cdp_page_crash_io_during_active_javascript"
    selected_scenarios = {
        name.strip()
        for name in os.environ.get("MOLI_INSPECTOR_ROUTING_SCENARIOS", "").split(",")
        if name.strip()
    }
    available_scenarios = {name for name, _scenario in page_scenarios} | {
        crash_scenario_name
    }
    unknown_scenarios = selected_scenarios.difference(available_scenarios)
    if unknown_scenarios:
        raise SmokeError(
            "Unknown Inspector routing scenario(s): "
            f"{sorted(unknown_scenarios)}; available={sorted(available_scenarios)}"
        )
    failures: list[tuple[str, Exception]] = []
    for scenario_name, scenario in page_scenarios:
        if selected_scenarios and scenario_name not in selected_scenarios:
            continue
        await _run_isolated_page_scenario(
            endpoint,
            fixture,
            results,
            failures,
            scenario_name,
            scenario,
        )
    if not selected_scenarios or crash_scenario_name in selected_scenarios:
        await _run_isolated_browser_scenario(
            endpoint,
            results,
            failures,
            crash_scenario_name,
            lambda client: _page_crash_io_during_active_javascript(
                client, fixture, results
            ),
        )
    if failures:
        details = "; ".join(
            f"{name}: {type(error).__name__}: {error}" for name, error in failures
        )
        raise SmokeError(f"Inspector routing contracts failed: {details}")


async def _run_isolated_page_scenario(
    endpoint: str,
    fixture: str,
    results: list[dict[str, Any]],
    failures: list[tuple[str, Exception]],
    scenario_name: str,
    scenario: Callable[[RawCdpClient, InspectorRoutingPage], Awaitable[None]],
) -> None:
    client = await connect_raw_cdp(endpoint)
    page: InspectorRoutingPage | None = None
    try:
        page = await _create_page(client, f"{fixture}/plain")
        await scenario(client, page)
    except Exception as error:
        failures.append((scenario_name, error))
        results.append(
            {
                "name": scenario_name,
                "ok": False,
                "error": f"{type(error).__name__}: {error}",
            }
        )
    finally:
        if page is not None:
            try:
                dispose_id = await client.send(
                    "Target.disposeBrowserContext",
                    {"browserContextId": page.browser_context_id},
                )
                await client.recv_until_id(dispose_id, timeout=3)
            except Exception:
                pass
        await _close_raw_cdp_client(client)


async def _run_isolated_browser_scenario(
    endpoint: str,
    results: list[dict[str, Any]],
    failures: list[tuple[str, Exception]],
    scenario_name: str,
    scenario: Callable[[RawCdpClient], Awaitable[None]],
) -> None:
    client = await connect_raw_cdp(endpoint)
    try:
        await scenario(client)
    except Exception as error:
        failures.append((scenario_name, error))
        results.append(
            {
                "name": scenario_name,
                "ok": False,
                "error": f"{type(error).__name__}: {error}",
            }
        )
    finally:
        await _close_raw_cdp_client(client)


async def _active_javascript_lane_matrix(
    client: RawCdpClient,
    page: InspectorRoutingPage,
    fixture: str,
    results: list[dict[str, Any]],
) -> None:
    await _reset_witness(fixture)
    source = """const xhr = new XMLHttpRequest();
xhr.open('GET', '/inspector-routing-witness/entered', false);
xhr.send();
globalThis.__inspectorRoutingLoopEntered =
  (globalThis.__inspectorRoutingLoopEntered || 0) + 1;
for (;;) {}"""
    compile_id = await client.send(
        "Runtime.compileScript",
        {
            "expression": source,
            "sourceURL": "inspector-routing-active.js",
            "persistScript": True,
        },
        session_id=page.primary_session_id,
    )
    compile_response, _ = await client.recv_until_id(compile_id, timeout=5)
    script_id = compile_response.get("result", {}).get("scriptId")
    if not isinstance(script_id, str) or not script_id:
        raise SmokeError(f"Runtime.compileScript returned no scriptId: {compile_response}")

    run_id = await client.send(
        "Runtime.runScript",
        {"scriptId": script_id},
        session_id=page.primary_session_id,
    )
    await _wait_for_witness(fixture, expected_count=1)

    main_one_id = await client.send(
        "Runtime.evaluate",
        {
            "expression": """(() => {
              if (globalThis.__inspectorRoutingLoopEntered !== 1)
                throw new Error('busy loop did not enter exactly once');
              return (globalThis.__inspectorRoutingMainOrder ??= []).push('m1');
            })()""",
            "returnByValue": True,
        },
        session_id=page.auxiliary_session_id,
    )
    main_two_id = await client.send(
        "Runtime.evaluate",
        {
            "expression": "globalThis.__inspectorRoutingMainOrder.push('m2')",
            "returnByValue": True,
        },
        session_id=page.auxiliary_session_id,
    )
    observed = await _recv_for(client, 0.2)
    blocked_ids = {run_id, main_one_id, main_two_id}
    early = [message for message in observed if message.get("id") in blocked_ids]
    if early:
        raise SmokeError(
            "active JavaScript and queued Main commands must remain blocked before IO: "
            f"{early}"
        )

    source_one_id = await client.send(
        "Debugger.getScriptSource",
        {"scriptId": script_id},
        session_id=page.auxiliary_session_id,
    )
    source_two_id = await client.send(
        "Debugger.getScriptSource",
        {"scriptId": script_id},
        session_id=page.auxiliary_session_id,
    )
    terminate_id = await client.send(
        "Runtime.terminateExecution",
        session_id=page.auxiliary_session_id,
    )
    expected_ids = {
        run_id,
        main_one_id,
        main_two_id,
        source_one_id,
        source_two_id,
        terminate_id,
    }
    responses, during_io = await _recv_responses(client, expected_ids, timeout=10)
    observed.extend(during_io)
    observed.extend(await _recv_for(client, 0.1))

    for message_id in expected_ids:
        count = sum(message.get("id") == message_id for message in observed)
        assert_equal(count, 1, f"exactly one Inspector routing response for id {message_id}")
    for message_id in (source_one_id, source_two_id):
        response = responses[message_id]
        if "error" in response:
            raise SmokeError(f"Debugger.getScriptSource failed during active JS: {response}")
        assert_equal(
            response.get("result", {}).get("scriptSource"),
            source,
            "active-JS IO Debugger.getScriptSource source",
        )
    terminate_response = responses[terminate_id]
    if "error" in terminate_response:
        raise SmokeError(
            f"Runtime.terminateExecution failed during active JS: {terminate_response}"
        )
    assert_equal(
        terminate_response.get("result"),
        {},
        "active-JS Runtime.terminateExecution result",
    )
    run_response = responses[run_id]
    if "error" not in run_response and not isinstance(
        run_response.get("result", {}).get("exceptionDetails"), dict
    ):
        raise SmokeError(f"terminated Runtime.runScript did not report termination: {run_response}")
    for message_id, expected_value in ((main_one_id, 1), (main_two_id, 2)):
        response = responses[message_id]
        if "error" in response:
            raise SmokeError(f"Main follower failed after termination: {response}")
        assert_equal(
            response.get("result", {}).get("result", {}).get("value"),
            expected_value,
            f"Main follower {message_id} value",
        )

    response_order = [
        message["id"]
        for message in observed
        if message.get("id") in expected_ids
    ]
    if response_order.index(source_one_id) >= response_order.index(source_two_id):
        raise SmokeError(
            "same-session IO source lookups must preserve response order: "
            f"{response_order}"
        )
    if response_order.index(main_one_id) >= response_order.index(main_two_id):
        raise SmokeError(
            f"same-session Main followers must preserve response order: {response_order}"
        )

    recovery_id = await client.send(
        "Runtime.evaluate",
        {
            "expression": "JSON.stringify({loop: __inspectorRoutingLoopEntered, order: __inspectorRoutingMainOrder})",
            "returnByValue": True,
        },
        session_id=page.primary_session_id,
    )
    recovery, _ = await client.recv_until_id(recovery_id, timeout=5)
    assert_equal(
        json.loads(recovery.get("result", {}).get("result", {}).get("value", "null")),
        {"loop": 1, "order": ["m1", "m2"]},
        "active-JS interrupt recovery state",
    )
    record_contract(
        results,
        "raw_cdp_active_javascript_main_io_lane_matrix",
        contract=(
            "An auxiliary IO lane interrupts non-yielding JavaScript owned by another "
            "session; IO and Main remain FIFO within their own lanes, each command responds "
            "once, and the isolate recovers."
        ),
        source="Chromium DevToolsSession Main/IO executable probe",
        commands=[
            "Runtime.runScript",
            "Runtime.evaluate x2",
            "Debugger.getScriptSource x2",
            "Runtime.terminateExecution",
        ],
        observed={"responseOrder": response_order},
    )


async def _worker_active_javascript_interrupt(
    client: RawCdpClient,
    page: InspectorRoutingPage,
    fixture: str,
    results: list[dict[str, Any]],
    *,
    target_type: str,
) -> None:
    if target_type == "worker":
        kind = "dedicated_worker"
        label = "DedicatedWorker"
        worker_path = "/worker.js?inspector-routing-active"
        auto_attach_session_id = page.primary_session_id
        create_source = f"""
            new Promise((resolve, reject) => {{
              const worker = new Worker({json.dumps(fixture + worker_path)}, {{
                name: 'inspector-routing-active-worker',
              }});
              globalThis.__inspectorRoutingWorker = worker;
              const timer = setTimeout(
                () => reject(new Error('DedicatedWorker ready timeout')),
                5000,
              );
              worker.onmessage = event => {{
                if (event.data && event.data.echoed === '__inspector_routing_worker_ready__') {{
                  clearTimeout(timer);
                  resolve(true);
                }}
              }};
              worker.onerror = event => {{
                clearTimeout(timer);
                reject(new Error(event.message || 'DedicatedWorker failed'));
              }};
              worker.postMessage('__inspector_routing_worker_ready__');
            }})
        """
    elif target_type == "shared_worker":
        kind = "shared_worker"
        label = "SharedWorker"
        worker_path = "/shared-worker.js?inspector-routing-active"
        auto_attach_session_id = None
        create_source = f"""
            new Promise((resolve, reject) => {{
              const worker = new SharedWorker(
                {json.dumps(fixture + worker_path)},
                'inspector-routing-active-shared-worker',
              );
              globalThis.__inspectorRoutingSharedWorker = worker;
              const timer = setTimeout(
                () => reject(new Error('SharedWorker ready timeout')),
                5000,
              );
              worker.port.onmessage = event => {{
                if (event.data && event.data.ready === true) {{
                  clearTimeout(timer);
                  resolve(true);
                }}
              }};
              worker.port.start();
              worker.port.postMessage({{kind: 'ready'}});
            }})
        """
    else:
        raise AssertionError(f"unsupported Worker target type: {target_type}")

    scenario_name = f"raw_cdp_{kind}_active_javascript_interrupt"
    await _reset_witness(fixture)
    auto_attach_id = await client.send(
        "Target.setAutoAttach",
        {
            "autoAttach": True,
            "waitForDebuggerOnStart": False,
            "flatten": True,
            "filter": [
                {"type": target_type, "exclude": False},
                {"exclude": True},
            ],
        },
        session_id=auto_attach_session_id,
    )
    await client.recv_until_id(auto_attach_id, timeout=5)

    create_id = await client.send(
        "Runtime.evaluate",
        {
            "expression": create_source,
            "awaitPromise": True,
            "returnByValue": True,
        },
        session_id=page.primary_session_id,
    )

    worker_session_id: str | None = None
    worker_target_id: str | None = None
    create_response: dict[str, Any] | None = None
    creation_seen: list[dict[str, Any]] = []
    deadline = asyncio.get_running_loop().time() + 10
    while create_response is None or worker_session_id is None:
        remaining = deadline - asyncio.get_running_loop().time()
        if remaining <= 0:
            raise SmokeError(
                f"timed out creating auto-attached {label}; "
                f"response={create_response} session={worker_session_id} "
                f"seen={creation_seen[-20:]}"
            )
        message = await asyncio.wait_for(client.recv(), timeout=remaining)
        creation_seen.append(message)
        if message.get("id") == create_id:
            create_response = message
        if message.get("method") != "Target.attachedToTarget":
            continue
        params = message.get("params", {})
        target_info = params.get("targetInfo", {})
        if target_info.get("type") != target_type:
            continue
        target_url = target_info.get("url")
        if not isinstance(target_url, str) or worker_path not in target_url:
            continue
        session_id = params.get("sessionId")
        target_id = target_info.get("targetId")
        if not isinstance(session_id, str) or not session_id:
            raise SmokeError(f"{label} attach event had no session id: {message}")
        if not isinstance(target_id, str) or not target_id:
            raise SmokeError(f"{label} attach event had no target id: {message}")
        worker_session_id = session_id
        worker_target_id = target_id

    assert create_response is not None
    assert worker_session_id is not None
    assert worker_target_id is not None
    if "error" in create_response:
        raise SmokeError(f"creating {label} failed: {create_response}")
    assert_equal(
        create_response.get("result", {}).get("result", {}).get("value"),
        True,
        f"{label} ready result",
    )

    runtime_enable_id = await client.send(
        "Runtime.enable",
        session_id=worker_session_id,
    )
    await client.recv_until_id(runtime_enable_id, timeout=5)

    busy_source = f"""(() => {{
  const xhr = new XMLHttpRequest();
  xhr.open('GET', {json.dumps(fixture + '/inspector-routing-witness/entered')}, false);
  xhr.send();
  globalThis.__inspectorRoutingWorkerLoopEntered =
    (globalThis.__inspectorRoutingWorkerLoopEntered || 0) + 1;
  for (;;) {{}}
}})()"""
    busy_id = await client.send(
        "Runtime.evaluate",
        {"expression": busy_source},
        session_id=worker_session_id,
    )
    await _wait_for_witness(fixture, expected_count=1)

    follower_one_id = await client.send(
        "Runtime.evaluate",
        {
            "expression": "(globalThis.__inspectorRoutingWorkerOrder ??= []).push('f1')",
            "returnByValue": True,
        },
        session_id=worker_session_id,
    )
    follower_two_id = await client.send(
        "Runtime.evaluate",
        {
            "expression": "globalThis.__inspectorRoutingWorkerOrder.push('f2')",
            "returnByValue": True,
        },
        session_id=worker_session_id,
    )
    blocked_ids = {busy_id, follower_one_id, follower_two_id}
    observed = await _recv_for(client, 0.2)
    early = [message for message in observed if message.get("id") in blocked_ids]
    if early:
        raise SmokeError(
            f"{label} Runtime.evaluate must not interrupt active JavaScript: "
            f"{early}"
        )

    terminate_id = await client.send(
        "Runtime.terminateExecution",
        session_id=worker_session_id,
    )
    expected_ids = blocked_ids | {terminate_id}
    responses, during_interrupt = await _recv_responses(
        client,
        expected_ids,
        timeout=10,
    )
    observed.extend(during_interrupt)
    observed.extend(await _recv_for(client, 0.1))

    for message_id in expected_ids:
        count = sum(message.get("id") == message_id for message in observed)
        assert_equal(
            count,
            1,
            f"exactly one {label} Inspector response for id {message_id}",
        )
    terminate_response = responses[terminate_id]
    if "error" in terminate_response:
        raise SmokeError(
            f"Runtime.terminateExecution failed to interrupt {label}: "
            f"{terminate_response}"
        )
    assert_equal(
        terminate_response.get("result"),
        {},
        f"{label} Runtime.terminateExecution result",
    )
    busy_response = responses[busy_id]
    if "error" not in busy_response and not isinstance(
        busy_response.get("result", {}).get("exceptionDetails"), dict
    ):
        raise SmokeError(
            f"terminated {label} Runtime.evaluate did not report termination: "
            f"{busy_response}"
        )
    for message_id, expected_value in (
        (follower_one_id, 1),
        (follower_two_id, 2),
    ):
        response = responses[message_id]
        if "error" in response:
            raise SmokeError(f"{label} follower failed after termination: {response}")
        assert_equal(
            response.get("result", {}).get("result", {}).get("value"),
            expected_value,
            f"{label} follower {message_id} value",
        )

    response_order = [
        message["id"] for message in observed if message.get("id") in expected_ids
    ]
    expected_response_order = [
        terminate_id,
        busy_id,
        follower_one_id,
        follower_two_id,
    ]
    if response_order != expected_response_order:
        raise SmokeError(
            f"{label} interrupt response must overtake the active evaluation before "
            "DontInterrupt followers resume in FIFO order: "
            f"expected={expected_response_order} actual={response_order}"
        )

    recovery_id = await client.send(
        "Runtime.evaluate",
        {
            "expression": "JSON.stringify({loop: __inspectorRoutingWorkerLoopEntered, order: __inspectorRoutingWorkerOrder})",
            "returnByValue": True,
        },
        session_id=worker_session_id,
    )
    recovery, _ = await client.recv_until_id(recovery_id, timeout=5)
    assert_equal(
        json.loads(recovery.get("result", {}).get("result", {}).get("value", "null")),
        {"loop": 1, "order": ["f1", "f2"]},
        f"{label} active-JS interrupt recovery state",
    )
    record_contract(
        results,
        scenario_name,
        contract=(
            "A Runtime.terminateExecution command overtakes earlier DontInterrupt evaluations "
            f"on the same {label} session, interrupts non-yielding JavaScript, then "
            "releases the queued evaluations in FIFO order with exactly-once responses."
        ),
        source=(
            "Chromium Worker DevTools IO-session executable probe; regression for Moli's "
            "former owner-only Worker Inspector queue"
        ),
        commands=[
            "Runtime.evaluate (non-yielding worker JavaScript)",
            "Runtime.evaluate x2 (DontInterrupt followers)",
            "Runtime.terminateExecution",
            "Runtime.evaluate (recovery)",
        ],
        observed={
            "targetId": worker_target_id,
            "responseOrder": response_order,
        },
    )


async def _debugger_io_catalog_during_active_javascript(
    client: RawCdpClient,
    page: InspectorRoutingPage,
    fixture: str,
    results: list[dict[str, Any]],
) -> None:
    await _reset_witness(fixture)
    source_url = "inspector-routing-debugger-io.js"
    source = """function inspectorRoutingHotLoop() {
  const xhr = new XMLHttpRequest();
  xhr.open('GET', '/inspector-routing-witness/entered', false);
  xhr.send();
  for (;;) {}
}
inspectorRoutingHotLoop();"""
    compile_id = await client.send(
        "Runtime.compileScript",
        {
            "expression": source,
            "sourceURL": source_url,
            "persistScript": True,
        },
        session_id=page.primary_session_id,
    )
    compiled, _ = await client.recv_until_id(compile_id, timeout=5)
    script_id = compiled.get("result", {}).get("scriptId")
    if not isinstance(script_id, str) or not script_id:
        raise SmokeError(f"Debugger IO probe compiled no script: {compiled}")

    run_id = await client.send(
        "Runtime.runScript",
        {"scriptId": script_id},
        session_id=page.primary_session_id,
    )
    await _wait_for_witness(fixture, expected_count=1)
    pause_id = await client.send(
        "Debugger.pause",
        session_id=page.auxiliary_session_id,
    )

    pause_response: dict[str, Any] | None = None
    paused_events: dict[str, dict[str, Any]] = {}
    pause_seen: list[dict[str, Any]] = []
    deadline = asyncio.get_running_loop().time() + 5.0
    while pause_response is None or paused_events.keys() != {
        page.primary_session_id,
        page.auxiliary_session_id,
    }:
        remaining = deadline - asyncio.get_running_loop().time()
        if remaining <= 0:
            raise SmokeError(
                "Debugger.pause did not interrupt active JavaScript on both sessions; "
                f"response={pause_response} sessions={paused_events.keys()} "
                f"seen={pause_seen[-20:]}"
            )
        message = await asyncio.wait_for(client.recv(), timeout=remaining)
        pause_seen.append(message)
        if message.get("id") == run_id:
            raise SmokeError(
                "Debugger.pause must not complete active JavaScript: "
                f"{message}"
            )
        if message.get("id") == pause_id:
            pause_response = message
        session_id = message.get("sessionId")
        if (
            session_id in {page.primary_session_id, page.auxiliary_session_id}
            and message.get("method") == "Debugger.paused"
        ):
            paused_events[session_id] = message
    if "error" in pause_response:
        raise SmokeError(f"Debugger.pause failed during active JS: {pause_response}")

    call_frames = paused_events[page.auxiliary_session_id].get("params", {}).get(
        "callFrames"
    )
    if not isinstance(call_frames, list) or not call_frames:
        raise SmokeError(f"Debugger.pause returned no call frame: {paused_events}")
    top_location = call_frames[0].get("location")
    if not isinstance(top_location, dict) or top_location.get("scriptId") != script_id:
        raise SmokeError(
            "Debugger.pause did not stop in the active probe script: "
            f"location={top_location} scriptId={script_id}"
        )

    catalog: list[tuple[str, dict[str, Any]]] = [
        (
            "Debugger.getPossibleBreakpoints",
            {
                "start": {
                    "scriptId": script_id,
                    "lineNumber": 0,
                    "columnNumber": 0,
                }
            },
        ),
        ("Debugger.getScriptSource", {"scriptId": script_id}),
        (
            "Debugger.getStackTrace",
            {"stackTraceId": {"id": "missing-stack", "debuggerId": "missing"}},
        ),
        (
            "Debugger.setBreakpoint",
            {
                "location": {
                    "scriptId": script_id,
                    "lineNumber": 0,
                    "columnNumber": 0,
                }
            },
        ),
        (
            "Debugger.setBreakpointByUrl",
            {"lineNumber": 0, "url": source_url},
        ),
        ("Debugger.setBreakpointsActive", {"active": False}),
        ("Debugger.setBreakpointsActive", {"active": True}),
    ]
    catalog_ids = [
        await client.send(method, params, session_id=page.auxiliary_session_id)
        for method, params in catalog
    ]
    catalog_responses, catalog_seen = await _recv_responses(
        client, set(catalog_ids), timeout=5
    )
    catalog_response_order = [
        message["id"] for message in catalog_seen if message.get("id") in catalog_ids
    ]
    assert_equal(
        catalog_response_order,
        catalog_ids,
        "Debugger IO catalog same-session response order",
    )
    source_response = catalog_responses[catalog_ids[1]]
    assert_equal(
        source_response.get("result", {}).get("scriptSource"),
        source,
        "Debugger IO catalog script source",
    )
    if "result" not in catalog_responses[catalog_ids[0]]:
        raise SmokeError(
            "Debugger.getPossibleBreakpoints failed while paused: "
            f"{catalog_responses[catalog_ids[0]]}"
        )
    # The fabricated async stack id is expected to fail, but the response must
    # cross the IO lane promptly rather than waiting for the page owner.
    if "error" not in catalog_responses[catalog_ids[2]]:
        raise SmokeError(
            "Debugger.getStackTrace unexpectedly accepted the fabricated stack id: "
            f"{catalog_responses[catalog_ids[2]]}"
        )

    breakpoint_ids: list[str] = []
    for message_id in catalog_ids[3:5]:
        response = catalog_responses[message_id]
        breakpoint_id = response.get("result", {}).get("breakpointId")
        if not isinstance(breakpoint_id, str) or not breakpoint_id:
            raise SmokeError(f"Debugger breakpoint command failed while paused: {response}")
        breakpoint_ids.append(breakpoint_id)
    remove_ids = [
        await client.send(
            "Debugger.removeBreakpoint",
            {"breakpointId": breakpoint_id},
            session_id=page.auxiliary_session_id,
        )
        for breakpoint_id in breakpoint_ids
    ]
    remove_responses, remove_seen = await _recv_responses(
        client, set(remove_ids), timeout=5
    )
    for message_id in remove_ids:
        if "error" in remove_responses[message_id]:
            raise SmokeError(
                f"Debugger.removeBreakpoint failed while paused: {remove_responses[message_id]}"
            )

    resume_id = await client.send(
        "Debugger.resume",
        session_id=page.auxiliary_session_id,
    )
    resume_response, resume_seen = await client.recv_until_id(resume_id, timeout=5)
    if "error" in resume_response:
        raise SmokeError(f"Debugger.resume failed in IO catalog probe: {resume_response}")
    if any(message.get("id") == run_id for message in resume_seen):
        raise SmokeError(
            "resuming an infinite script must not complete it: "
            f"{resume_seen}"
        )
    resumed_sessions = {
        message.get("sessionId")
        for message in resume_seen
        if message.get("method") == "Debugger.resumed"
    }
    for session_id in (page.primary_session_id, page.auxiliary_session_id):
        if session_id not in resumed_sessions:
            resume_seen.append(
                await _recv_until_session_event(
                    client,
                    session_id,
                    "Debugger.resumed",
                    timeout=2,
                )
            )
            resumed_sessions.add(session_id)

    follower_id = await client.send(
        "Runtime.evaluate",
        {"expression": "21 * 2", "returnByValue": True},
        session_id=page.auxiliary_session_id,
    )
    early_after_resume = await _recv_for(client, 0.1)
    if any(message.get("id") in {run_id, follower_id} for message in early_after_resume):
        raise SmokeError(
            "active JavaScript and its Main follower must remain blocked after resume: "
            f"{early_after_resume}"
        )

    terminate_id = await client.send(
        "Runtime.terminateExecution",
        session_id=page.auxiliary_session_id,
    )
    final_responses, final_seen = await _recv_responses(
        client,
        {run_id, follower_id, terminate_id},
        timeout=10,
    )
    assert_equal(
        final_responses[follower_id]
        .get("result", {})
        .get("result", {})
        .get("value"),
        42,
        "Main recovery after Debugger pause/resume/terminate",
    )
    record_contract(
        results,
        "raw_cdp_debugger_io_catalog_during_active_javascript",
        contract=(
            "The Chromium Debugger IO method family interrupts active JavaScript, remains "
            "FIFO and usable inside the pause loop, resumes independently of Main, and "
            "leaves the target recoverable after termination."
        ),
        source="Chromium DevToolsSession::ShouldSendOnIO executable probe",
        commands=[method for method, _params in catalog]
        + ["Debugger.removeBreakpoint", "Debugger.resume", "Runtime.terminateExecution"],
        observed={
            "catalogResponseOrder": catalog_response_order,
            "removeResponseOrder": [
                message["id"]
                for message in remove_seen
                if message.get("id") in remove_ids
            ],
            "pauseMessages": len(pause_seen),
            "resumeMessages": len(resume_seen),
            "messagesWhileResumed": len(early_after_resume),
            "terminationMessages": len(final_seen),
        },
    )


async def _nested_v8_main_receiver_matrix(
    client: RawCdpClient,
    page: InspectorRoutingPage,
    results: list[dict[str, Any]],
) -> None:
    dom_enable_id = await client.send(
        "DOM.enable",
        session_id=page.auxiliary_session_id,
    )
    await client.recv_until_id(dom_enable_id, timeout=5)
    frame_tree_id = await client.send(
        "Page.getFrameTree",
        session_id=page.auxiliary_session_id,
    )
    frame_tree, _ = await client.recv_until_id(frame_tree_id, timeout=5)
    frame_id = (
        frame_tree.get("result", {})
        .get("frameTree", {})
        .get("frame", {})
        .get("id")
    )
    if not isinstance(frame_id, str) or not frame_id:
        raise SmokeError(f"Page.getFrameTree returned no top frame id: {frame_tree}")
    isolated_id = await client.send(
        "Page.createIsolatedWorld",
        {"frameId": frame_id, "worldName": "inspector-routing-nested-main"},
        session_id=page.auxiliary_session_id,
    )
    isolated, _ = await client.recv_until_id(isolated_id, timeout=5)
    isolated_context_id = isolated.get("result", {}).get("executionContextId")
    if not isinstance(isolated_context_id, int) or isolated_context_id <= 0:
        raise SmokeError(f"Page.createIsolatedWorld returned no context id: {isolated}")

    paused_evaluate_id = await client.send(
        "Runtime.evaluate",
        {"expression": "debugger; 42", "returnByValue": True},
        session_id=page.primary_session_id,
    )
    paused_events: dict[str, dict[str, Any]] = {}
    seen: list[dict[str, Any]] = []
    deadline = asyncio.get_running_loop().time() + 5.0
    while paused_events.keys() != {
        page.primary_session_id,
        page.auxiliary_session_id,
    }:
        remaining = deadline - asyncio.get_running_loop().time()
        if remaining <= 0:
            raise SmokeError(
                "timed out waiting for both Debugger.paused events; "
                f"sessions={paused_events.keys()} seen={seen[-20:]}"
            )
        message = await asyncio.wait_for(client.recv(), timeout=remaining)
        seen.append(message)
        if message.get("id") == paused_evaluate_id:
            raise SmokeError("debugger evaluation completed before Debugger.paused")
        session_id = message.get("sessionId")
        if (
            session_id in {page.primary_session_id, page.auxiliary_session_id}
            and message.get("method") == "Debugger.paused"
        ):
            paused_events[session_id] = message

    call_frames = paused_events[page.auxiliary_session_id].get("params", {}).get(
        "callFrames"
    )
    if not isinstance(call_frames, list) or not call_frames:
        raise SmokeError(
            "auxiliary Debugger.paused event has no call frame: "
            f"{paused_events[page.auxiliary_session_id]}"
        )
    call_frame_id = call_frames[0].get("callFrameId")
    if not isinstance(call_frame_id, str) or not call_frame_id:
        raise SmokeError(f"auxiliary pause has no callFrameId: {call_frames[0]}")

    fifo_ids = [
        await client.send(
            "Runtime.evaluate",
            {
                "expression": (
                    f"(globalThis.__nestedMainOrder ??= []).push('n{index}')"
                ),
                "returnByValue": True,
            },
            session_id=page.auxiliary_session_id,
        )
        for index in range(1, 4)
    ]
    fifo_responses, fifo_seen = await _recv_responses(client, set(fifo_ids), timeout=5)
    fifo_response_order = [
        message["id"] for message in fifo_seen if message.get("id") in fifo_ids
    ]
    assert_equal(fifo_response_order, fifo_ids, "nested Main FIFO response order")
    for index, message_id in enumerate(fifo_ids, start=1):
        assert_equal(
            fifo_responses[message_id]
            .get("result", {})
            .get("result", {})
            .get("value"),
            index,
            f"nested Main FIFO side effect {index}",
        )

    object_id = await client.send(
        "Runtime.evaluate",
        {"expression": "({answer: 42})"},
        session_id=page.auxiliary_session_id,
    )
    object_response, _ = await client.recv_until_id(object_id, timeout=5)
    remote_object_id = (
        object_response.get("result", {}).get("result", {}).get("objectId")
    )
    if not isinstance(remote_object_id, str) or not remote_object_id:
        raise SmokeError(f"nested Main object evaluate returned no objectId: {object_response}")

    properties_id = await client.send(
        "Runtime.getProperties",
        {"objectId": remote_object_id, "ownProperties": True},
        session_id=page.auxiliary_session_id,
    )
    properties, _ = await client.recv_until_id(properties_id, timeout=5)
    descriptors = properties.get("result", {}).get("result", [])
    if not any(
        descriptor.get("name") == "answer"
        and descriptor.get("value", {}).get("value") == 42
        for descriptor in descriptors
        if isinstance(descriptor, dict)
    ):
        raise SmokeError(f"nested Main getProperties missed answer=42: {properties}")

    call_id = await client.send(
        "Runtime.callFunctionOn",
        {
            "objectId": remote_object_id,
            "functionDeclaration": "function () { return this.answer + 1; }",
            "returnByValue": True,
        },
        session_id=page.auxiliary_session_id,
    )
    called, _ = await client.recv_until_id(call_id, timeout=5)
    assert_equal(
        called.get("result", {}).get("result", {}).get("value"),
        43,
        "nested Main object-targeted callFunctionOn",
    )

    frame_evaluate_id = await client.send(
        "Debugger.evaluateOnCallFrame",
        {
            "callFrameId": call_frame_id,
            "expression": "40 + 2",
            "returnByValue": True,
        },
        session_id=page.auxiliary_session_id,
    )
    frame_evaluate, _ = await client.recv_until_id(frame_evaluate_id, timeout=5)
    assert_equal(
        frame_evaluate.get("result", {}).get("result", {}).get("value"),
        42,
        "nested Main Debugger.evaluateOnCallFrame",
    )

    context_evaluate_id = await client.send(
        "Runtime.evaluate",
        {
            "contextId": isolated_context_id,
            "expression": "globalThis.__nestedExplicitContext = 44",
            "returnByValue": True,
        },
        session_id=page.auxiliary_session_id,
    )
    context_evaluate, _ = await client.recv_until_id(context_evaluate_id, timeout=5)
    assert_equal(
        context_evaluate.get("result", {}).get("result", {}).get("value"),
        44,
        "nested Main explicit-context Runtime.evaluate",
    )

    resume_id = await client.send(
        "Debugger.resume",
        session_id=page.auxiliary_session_id,
    )
    responses, resume_seen = await _recv_responses(
        client,
        {resume_id, paused_evaluate_id},
        timeout=5,
    )
    if "error" in responses[resume_id]:
        raise SmokeError(f"auxiliary Debugger.resume failed: {responses[resume_id]}")
    assert_equal(
        responses[paused_evaluate_id]
        .get("result", {})
        .get("result", {})
        .get("value"),
        42,
        "nested Main original evaluation after auxiliary resume",
    )
    resumed_sessions = {
        message.get("sessionId")
        for message in resume_seen
        if message.get("method") == "Debugger.resumed"
    }
    if page.primary_session_id not in resumed_sessions:
        extra = await _recv_until_session_event(
            client,
            page.primary_session_id,
            "Debugger.resumed",
            timeout=2,
        )
        resume_seen.append(extra)
        resumed_sessions.add(page.primary_session_id)

    record_contract(
        results,
        "raw_cdp_nested_v8_main_receiver_matrix",
        contract=(
            "A normal debugger pause pumps V8-backed commands from the Main DevTools "
            "receiver for every attached session, including explicit-context Runtime and "
            "call-frame/object commands; an auxiliary IO resume releases the original "
            "JavaScript stack."
        ),
        source="Chromium nested main-thread RunLoop executable probe",
        commands=[
            "Runtime.evaluate",
            "Runtime.getProperties",
            "Runtime.callFunctionOn",
            "Debugger.evaluateOnCallFrame",
            "Debugger.resume",
        ],
        observed={
            "fifoResponseOrder": fifo_response_order,
            "resumedSessions": sorted(
                session
                for session in resumed_sessions
                if isinstance(session, str)
            ),
        },
    )


async def _nested_non_v8_main_receiver_matrix(
    client: RawCdpClient,
    page: InspectorRoutingPage,
    results: list[dict[str, Any]],
) -> None:
    dom_enable_id = await client.send(
        "DOM.enable",
        session_id=page.auxiliary_session_id,
    )
    await client.recv_until_id(dom_enable_id, timeout=5)
    initial_tree_id = await client.send(
        "Page.getFrameTree",
        session_id=page.auxiliary_session_id,
    )
    initial_tree, _ = await client.recv_until_id(initial_tree_id, timeout=5)
    frame_id = (
        initial_tree.get("result", {})
        .get("frameTree", {})
        .get("frame", {})
        .get("id")
    )
    if not isinstance(frame_id, str) or not frame_id:
        raise SmokeError(f"Page.getFrameTree returned no top frame id: {initial_tree}")

    paused_evaluate_id = await client.send(
        "Runtime.evaluate",
        {"expression": "debugger; 84", "returnByValue": True},
        session_id=page.primary_session_id,
    )
    await _recv_paused_for_both_sessions(client, page, paused_evaluate_id)

    commands = [
        (
            "Runtime.evaluate",
            {
                "expression": (
                    "(globalThis.__nestedMixedMainOrder ??= []).push('v8-1')"
                ),
                "returnByValue": True,
            },
        ),
        ("Page.getFrameTree", None),
        (
            "Runtime.evaluate",
            {
                "expression": "globalThis.__nestedMixedMainOrder.push('v8-2')",
                "returnByValue": True,
            },
        ),
        ("Page.getLayoutMetrics", None),
        (
            "Runtime.evaluate",
            {
                "expression": "globalThis.__nestedMixedMainOrder.push('v8-3')",
                "returnByValue": True,
            },
        ),
        ("DOM.getDocument", {"depth": 0}),
    ]
    command_ids = [
        await client.send(method, params, session_id=page.auxiliary_session_id)
        for method, params in commands
    ]
    seen = await _recv_for(client, 1.0)
    responses: dict[int, dict[str, Any]] = {}
    for message in seen:
        message_id = message.get("id")
        if message_id in command_ids:
            if message_id in responses:
                raise SmokeError(
                    f"nested Main receiver duplicated response {message_id}: {seen}"
                )
            responses[message_id] = message
    missing_before_resume = set(command_ids).difference(responses)

    resume_id = await client.send(
        "Debugger.resume",
        session_id=page.auxiliary_session_id,
    )
    after_resume, resume_seen = await _recv_responses(
        client,
        {resume_id, paused_evaluate_id, *missing_before_resume},
        timeout=5,
    )
    for message_id in missing_before_resume:
        responses[message_id] = after_resume[message_id]
    assert_equal(
        after_resume[paused_evaluate_id]
        .get("result", {})
        .get("result", {})
        .get("value"),
        84,
        "nested non-V8 Main original evaluation after resume",
    )
    if missing_before_resume:
        missing_methods = [
            method
            for (method, _params), message_id in zip(commands, command_ids, strict=True)
            if message_id in missing_before_resume
        ]
        raise SmokeError(
            "normal debugger pause did not pump non-V8 Main commands before resume: "
            f"{missing_methods}"
        )

    response_order = [
        message["id"] for message in seen if message.get("id") in command_ids
    ]
    assert_equal(
        response_order,
        command_ids,
        "nested mixed V8/non-V8 Main FIFO response order",
    )

    for (method, _params), message_id in zip(commands, command_ids, strict=True):
        if "error" in responses[message_id]:
            raise SmokeError(
                f"nested Main receiver did not dispatch {method}: {responses[message_id]}"
            )
    for command_index, expected_value in zip((0, 2, 4), (1, 2, 3), strict=True):
        assert_equal(
            responses[command_ids[command_index]]
            .get("result", {})
            .get("result", {})
            .get("value"),
            expected_value,
            f"nested mixed Main V8 side effect {expected_value}",
        )
    nested_frame_id = (
        responses[command_ids[1]]
        .get("result", {})
        .get("frameTree", {})
        .get("frame", {})
        .get("id")
    )
    assert_equal(nested_frame_id, frame_id, "nested Main Page.getFrameTree frame")
    nested_root = responses[command_ids[5]].get("result", {}).get("root")
    if not isinstance(nested_root, dict) or nested_root.get("nodeType") != 9:
        raise SmokeError(
            "nested Main DOM.getDocument returned no document root: "
            f"{responses[command_ids[5]]}"
        )

    record_contract(
        results,
        "raw_cdp_nested_non_v8_main_receiver_matrix",
        contract=(
            "A normal debugger pause pumps V8, Page and DOM agents through one Main "
            "DevTools receiver and one per-session FIFO lane; mixed commands settle in "
            "send order before Debugger.resume, while the original JavaScript stack waits."
        ),
        source="Chromium nested main-thread RunLoop executable probe",
        commands=[method for method, _params in commands] + ["Debugger.resume"],
        observed={
            "responseOrder": response_order,
            "resumeMessages": len(resume_seen),
        },
    )


async def _performance_io_during_active_javascript(
    client: RawCdpClient,
    page: InspectorRoutingPage,
    fixture: str,
    results: list[dict[str, Any]],
) -> None:
    performance_enable_id = await client.send(
        "Performance.enable",
        session_id=page.auxiliary_session_id,
    )
    await client.recv_until_id(performance_enable_id, timeout=5)
    await _reset_witness(fixture)
    busy_id = await client.send(
        "Runtime.evaluate",
        {
            "expression": """(() => {
              const xhr = new XMLHttpRequest();
              xhr.open('GET', '/inspector-routing-witness/entered', false);
              xhr.send();
              globalThis.__performanceIoLoopEntered = true;
              for (;;) {}
            })()""",
            "returnByValue": True,
        },
        session_id=page.primary_session_id,
    )
    await _wait_for_witness(fixture, expected_count=1)
    follower_id = await client.send(
        "Runtime.evaluate",
        {"expression": "6 * 7", "returnByValue": True},
        session_id=page.auxiliary_session_id,
    )
    metrics_id = await client.send(
        "Performance.getMetrics",
        session_id=page.auxiliary_session_id,
    )
    metrics, before_terminate = await client.recv_until_id(metrics_id, timeout=5)
    if any(message.get("id") in {busy_id, follower_id} for message in before_terminate):
        raise SmokeError(
            "Performance.getMetrics must not release active JS or its Main follower: "
            f"{before_terminate}"
        )
    metric_values = metrics.get("result", {}).get("metrics")
    if not isinstance(metric_values, list) or not metric_values:
        raise SmokeError(
            f"Performance.getMetrics returned no live metrics during active JS: {metrics}"
        )
    metrics_by_name = {
        metric.get("name"): metric.get("value")
        for metric in metric_values
        if isinstance(metric, dict)
        and isinstance(metric.get("name"), str)
        and isinstance(metric.get("value"), (int, float))
    }
    for metric_name in ("Documents", "Frames", "Nodes"):
        metric_value = metrics_by_name.get(metric_name)
        if not isinstance(metric_value, (int, float)) or metric_value <= 0:
            raise SmokeError(
                "Performance.getMetrics returned no positive live "
                f"{metric_name} metric during active JS: {metrics_by_name}"
            )
    heap_used = metrics_by_name.get("JSHeapUsedSize")
    if not isinstance(heap_used, (int, float)) or heap_used < 0:
        raise SmokeError(
            "Performance.getMetrics returned no non-negative JSHeapUsedSize metric "
            f"during active JS: {metrics_by_name}"
        )

    terminate_id = await client.send(
        "Runtime.terminateExecution",
        session_id=page.auxiliary_session_id,
    )
    responses, after_terminate = await _recv_responses(
        client,
        {busy_id, follower_id, terminate_id},
        timeout=10,
    )
    assert_equal(
        responses[follower_id].get("result", {}).get("result", {}).get("value"),
        42,
        "Main follower after Performance IO and termination",
    )
    performance_disable_id = await client.send(
        "Performance.disable",
        session_id=page.auxiliary_session_id,
    )
    await client.recv_until_id(performance_disable_id, timeout=5)
    record_contract(
        results,
        "raw_cdp_performance_io_during_active_javascript",
        contract=(
            "Chromium's non-V8 Performance.getMetrics IO command completes while "
            "non-yielding JavaScript and a Main follower remain blocked."
        ),
        source="Chromium DevToolsSession::ShouldSendOnIO executable probe",
        commands=[
            "Runtime.evaluate",
            "Performance.getMetrics",
            "Runtime.terminateExecution",
        ],
        observed={
            "metricCount": len(metric_values),
            "liveMetrics": {
                name: metrics_by_name[name]
                for name in ("Documents", "Frames", "Nodes", "JSHeapUsedSize")
            },
            "messagesBeforeTerminate": len(before_terminate),
            "messagesAfterTerminate": len(after_terminate),
        },
    )


async def _script_execution_disabled_io_during_active_javascript(
    client: RawCdpClient,
    page: InspectorRoutingPage,
    fixture: str,
    results: list[dict[str, Any]],
) -> None:
    await _reset_witness(fixture)
    busy_id = await client.send(
        "Runtime.evaluate",
        {
            "expression": """(() => {
              const xhr = new XMLHttpRequest();
              xhr.open('GET', '/inspector-routing-witness/entered', false);
              xhr.send();
              for (;;) {}
            })()""",
            "returnByValue": True,
        },
        session_id=page.primary_session_id,
    )
    await _wait_for_witness(fixture, expected_count=1)
    follower_id = await client.send(
        "Runtime.evaluate",
        {"expression": "40 + 2", "returnByValue": True},
        session_id=page.auxiliary_session_id,
    )

    disable_id = await client.send(
        "Emulation.setScriptExecutionDisabled",
        {"value": True},
        session_id=page.auxiliary_session_id,
    )
    disable_response, disable_seen = await client.recv_until_id(disable_id, timeout=5)
    if "error" in disable_response:
        raise SmokeError(
            "Emulation.setScriptExecutionDisabled(true) failed during active JS: "
            f"{disable_response}"
        )
    if any(message.get("id") in {busy_id, follower_id} for message in disable_seen):
        raise SmokeError(
            "disabling future script execution must not complete the active script or Main "
            f"follower: {disable_seen}"
        )

    enable_id = await client.send(
        "Emulation.setScriptExecutionDisabled",
        {"value": False},
        session_id=page.auxiliary_session_id,
    )
    enable_response, enable_seen = await client.recv_until_id(enable_id, timeout=5)
    if "error" in enable_response:
        raise SmokeError(
            "Emulation.setScriptExecutionDisabled(false) failed during active JS: "
            f"{enable_response}"
        )
    if any(message.get("id") in {busy_id, follower_id} for message in enable_seen):
        raise SmokeError(
            "reenabling future script execution must not complete the active script or Main "
            f"follower: {enable_seen}"
        )

    terminate_id = await client.send(
        "Runtime.terminateExecution",
        session_id=page.auxiliary_session_id,
    )
    responses, termination_seen = await _recv_responses(
        client,
        {busy_id, follower_id, terminate_id},
        timeout=10,
    )
    assert_equal(
        responses[follower_id].get("result", {}).get("result", {}).get("value"),
        42,
        "Main recovery after script-execution IO toggle",
    )
    record_contract(
        results,
        "raw_cdp_script_execution_disabled_io_during_active_javascript",
        contract=(
            "Emulation.setScriptExecutionDisabled crosses Chromium's non-V8 IO agent "
            "boundary in both directions while active JavaScript and Main remain blocked, "
            "without confusing the setting with execution termination."
        ),
        source="Chromium DevToolsSession::ShouldSendOnIO executable probe",
        commands=[
            "Runtime.evaluate",
            "Emulation.setScriptExecutionDisabled(true)",
            "Emulation.setScriptExecutionDisabled(false)",
            "Runtime.terminateExecution",
        ],
        observed={
            "disableMessages": len(disable_seen),
            "enableMessages": len(enable_seen),
            "terminationMessages": len(termination_seen),
        },
    )


async def _instrumentation_pause_io_only_receiver(
    client: RawCdpClient,
    page: InspectorRoutingPage,
    results: list[dict[str, Any]],
) -> None:
    source = """globalThis.__instrumentationPauseRan = 21 * 2;
//# sourceURL=inspector-routing-instrumentation.js"""

    breakpoint_id = await client.send(
        "Debugger.setInstrumentationBreakpoint",
        {"instrumentation": "beforeScriptExecution"},
        session_id=page.auxiliary_session_id,
    )
    breakpoint_response, _ = await client.recv_until_id(breakpoint_id, timeout=5)
    instrumentation_breakpoint_id = breakpoint_response.get("result", {}).get(
        "breakpointId"
    )
    if not isinstance(instrumentation_breakpoint_id, str) or not instrumentation_breakpoint_id:
        raise SmokeError(
            "Debugger.setInstrumentationBreakpoint returned no breakpoint id: "
            f"{breakpoint_response}"
        )

    run_id = await client.send(
        "Runtime.evaluate",
        {"expression": source, "returnByValue": True},
        session_id=page.auxiliary_session_id,
    )
    paused_events = await _recv_paused_for_both_sessions(client, page, run_id)
    auxiliary_pause = paused_events[page.auxiliary_session_id]
    assert_equal(
        auxiliary_pause.get("params", {}).get("reason"),
        "instrumentation",
        "instrumentation pause reason",
    )
    call_frames = auxiliary_pause.get("params", {}).get("callFrames")
    script_id = (
        call_frames[0].get("location", {}).get("scriptId")
        if isinstance(call_frames, list)
        and call_frames
        and isinstance(call_frames[0], dict)
        else None
    )
    if not isinstance(script_id, str) or not script_id:
        raise SmokeError(
            f"instrumentation pause returned no script id: {auxiliary_pause}"
        )

    main_follower_id = await client.send(
        "Runtime.evaluate",
        {"expression": "globalThis.__instrumentationPauseRan + 1", "returnByValue": True},
        session_id=page.primary_session_id,
    )
    blocked = await _recv_for(client, 0.2)
    early_command_responses = {
        message["id"]: message
        for message in blocked
        if message.get("id") in {run_id, main_follower_id}
    }

    source_id = await client.send(
        "Debugger.getScriptSource",
        {"scriptId": script_id},
        session_id=page.auxiliary_session_id,
    )
    remove_id = await client.send(
        "Debugger.removeBreakpoint",
        {"breakpointId": instrumentation_breakpoint_id},
        session_id=page.auxiliary_session_id,
    )
    io_responses, io_seen = await _recv_responses(
        client,
        {source_id, remove_id},
        timeout=5,
    )
    assert_equal(
        io_responses[source_id].get("result", {}).get("scriptSource"),
        source,
        "instrumentation-pause IO script source",
    )
    if "error" in io_responses[remove_id]:
        raise SmokeError(
            "Debugger.removeBreakpoint failed in instrumentation pause: "
            f"{io_responses[remove_id]}"
        )

    resume_id = await client.send(
        "Debugger.resume",
        session_id=page.auxiliary_session_id,
    )
    pending_after_resume = {run_id, main_follower_id}.difference(
        early_command_responses
    )
    completed, resume_seen = await _recv_responses(
        client,
        {resume_id, *pending_after_resume},
        timeout=5,
    )
    completed.update(early_command_responses)
    if "error" in completed[resume_id]:
        raise SmokeError(
            f"Debugger.resume failed after instrumentation pause: {completed[resume_id]}"
        )
    if "error" in completed[run_id]:
        raise SmokeError(
            f"instrumented Runtime.runScript failed after resume: {completed[run_id]}"
        )
    if early_command_responses:
        raise SmokeError(
            "an instrumentation pause must not pump the normal Main receiver; "
            f"early response ids={sorted(early_command_responses)}"
        )
    assert_equal(
        completed[main_follower_id]
        .get("result", {})
        .get("result", {})
        .get("value"),
        43,
        "Main follower after instrumentation-pause IO resume",
    )
    record_contract(
        results,
        "raw_cdp_instrumentation_pause_io_only_receiver",
        contract=(
            "An instrumentation pause does not run Chromium's nestable Main receiver. "
            "Main stays blocked, while the IO Debugger receiver can inspect the script, "
            "remove the breakpoint, and resume the isolate."
        ),
        source="Chromium WebDevToolsAgentImpl instrumentation-pause executable probe",
        commands=[
            "Debugger.setInstrumentationBreakpoint",
            "Runtime.evaluate (instrumented)",
            "Runtime.evaluate",
            "Debugger.getScriptSource",
            "Debugger.removeBreakpoint",
            "Debugger.resume",
        ],
        observed={
            "pauseReason": auxiliary_pause.get("params", {}).get("reason"),
            "messagesWhileMainBlocked": len(blocked),
            "ioMessages": len(io_seen),
            "resumeMessages": len(resume_seen),
        },
    )


async def _navigation_replacement_during_active_javascript(
    client: RawCdpClient,
    page: InspectorRoutingPage,
    fixture: str,
    results: list[dict[str, Any]],
) -> None:
    await _reset_witness(fixture)
    source = "globalThis.__inspectorRoutingNavigationMarker = 1;"
    compile_id = await client.send(
        "Runtime.compileScript",
        {
            "expression": source,
            "sourceURL": "inspector-routing-navigation.js",
            "persistScript": True,
        },
        session_id=page.primary_session_id,
    )
    compiled, _ = await client.recv_until_id(compile_id, timeout=5)
    script_id = compiled.get("result", {}).get("scriptId")
    if not isinstance(script_id, str) or not script_id:
        raise SmokeError(f"navigation probe compiled no script: {compiled}")
    marker_run_id = await client.send(
        "Runtime.runScript",
        {"scriptId": script_id},
        session_id=page.primary_session_id,
    )
    marker_run, _ = await client.recv_until_id(marker_run_id, timeout=5)
    assert_equal(
        marker_run.get("result", {}).get("result", {}).get("value"),
        1,
        "navigation replacement marker script",
    )
    busy_id = await client.send(
        "Runtime.evaluate",
        {
            "expression": """(() => {
              const xhr = new XMLHttpRequest();
              xhr.open('GET', '/inspector-routing-witness/entered', false);
              xhr.send();
              for (;;) {}
            })()""",
            "returnByValue": True,
        },
        session_id=page.primary_session_id,
    )
    await _wait_for_witness(fixture, expected_count=1)

    destination = f"{fixture}/plain?inspector-routing-generation=2"
    navigate_id = await client.send(
        "Page.navigate",
        {"url": destination},
        session_id=page.auxiliary_session_id,
    )
    source_id = await client.send(
        "Debugger.getScriptSource",
        {"scriptId": script_id},
        session_id=page.auxiliary_session_id,
    )
    terminate_id = await client.send(
        "Runtime.terminateExecution",
        session_id=page.auxiliary_session_id,
    )

    expected_ids = {busy_id, navigate_id, source_id, terminate_id}
    responses: dict[int, dict[str, Any]] = {}
    seen: list[dict[str, Any]] = []
    saw_load = False
    deadline = asyncio.get_running_loop().time() + 10.0
    while responses.keys() != expected_ids or not saw_load:
        remaining = deadline - asyncio.get_running_loop().time()
        if remaining <= 0:
            raise SmokeError(
                "navigation replacement did not recover after IO termination; "
                f"responses={responses} sawLoad={saw_load} seen={seen[-30:]}"
            )
        message = await asyncio.wait_for(client.recv(), timeout=remaining)
        seen.append(message)
        message_id = message.get("id")
        if message_id in expected_ids:
            if message_id in responses:
                raise SmokeError(
                    f"duplicate navigation replacement response {message_id}: {seen[-20:]}"
                )
            responses[message_id] = message
        if (
            message.get("sessionId") == page.primary_session_id
            and message.get("method") == "Page.loadEventFired"
        ):
            saw_load = True

    source_error = responses[source_id].get("error")
    if source_error is None:
        assert_equal(
            responses[source_id].get("result", {}).get("scriptSource"),
            source,
            "navigation replacement old-attachment script source",
        )
        source_outcome = "old-attachment-source"
    elif (
        isinstance(source_error, dict)
        and source_error.get("code") == -32000
        and isinstance(source_error.get("message"), str)
        and (
            source_error["message"].startswith("No script for id: ")
            or source_error["message"] == "Inspected target navigated or closed"
        )
    ):
        source_outcome = "replacement-error"
    else:
        raise SmokeError(
            "Debugger.getScriptSource returned an unexpected replacement error: "
            f"{responses[source_id]}"
        )
    if "error" in responses[navigate_id]:
        raise SmokeError(f"Page.navigate failed after termination: {responses[navigate_id]}")
    terminate_error = responses[terminate_id].get("error")
    if terminate_error is not None and terminate_error != {
        "code": -32000,
        "message": "Inspected target navigated or closed",
    }:
        raise SmokeError(
            "Runtime.terminateExecution returned an unexpected replacement error: "
            f"{responses[terminate_id]}"
        )
    recovery_id = await client.send(
        "Runtime.evaluate",
        {"expression": "location.href", "returnByValue": True},
        session_id=page.primary_session_id,
    )
    recovered, _ = await client.recv_until_id(recovery_id, timeout=5)
    assert_equal(
        recovered.get("result", {}).get("result", {}).get("value"),
        destination,
        "navigation replacement destination",
    )
    record_contract(
        results,
        "raw_cdp_navigation_replacement_during_active_javascript",
        contract=(
            "A pending Main navigation does not suspend the IO lane. Source lookup and "
            "termination both settle, while their target, result and response order may race "
            "the unordered Main replacement and use Chromium's exact replacement errors. The "
            "navigation commits and the replacement Inspector session remains live."
        ),
        source="Chromium ShouldSuspendDuringNavigation executable probe",
        commands=[
            "Runtime.runScript",
            "Runtime.evaluate",
            "Page.navigate",
            "Debugger.getScriptSource",
            "Runtime.terminateExecution",
            "Runtime.evaluate",
        ],
        observed={
            "responseOrder": [
                message["id"]
                for message in seen
                if message.get("id") in expected_ids
            ],
            "loadEvent": saw_load,
            "sourceOutcome": source_outcome,
            "terminateOutcome": "replacement-error"
            if terminate_error is not None
            else "success",
        },
    )


async def _session_detach_with_interrupts_in_flight(
    client: RawCdpClient,
    page: InspectorRoutingPage,
    fixture: str,
    results: list[dict[str, Any]],
) -> None:
    await _reset_witness(fixture)
    source = """const xhr = new XMLHttpRequest();
xhr.open('GET', '/inspector-routing-witness/entered', false);
xhr.send();
for (;;) {}"""
    compile_id = await client.send(
        "Runtime.compileScript",
        {
            "expression": source,
            "sourceURL": "inspector-routing-detach.js",
            "persistScript": True,
        },
        session_id=page.primary_session_id,
    )
    compiled, _ = await client.recv_until_id(compile_id, timeout=5)
    script_id = compiled.get("result", {}).get("scriptId")
    if not isinstance(script_id, str) or not script_id:
        raise SmokeError(f"detach-race probe compiled no script: {compiled}")

    run_id = await client.send(
        "Runtime.runScript",
        {"scriptId": script_id},
        session_id=page.primary_session_id,
    )
    await _wait_for_witness(fixture, expected_count=1)

    interrupt_ids = [
        await client.send(
            "Debugger.getScriptSource",
            {"scriptId": script_id},
            session_id=page.auxiliary_session_id,
        )
        for _ in range(32)
    ]
    detach_id = await client.send(
        "Target.detachFromTarget",
        {"sessionId": page.auxiliary_session_id},
    )
    detach_completed = False
    try:
        detach_response, seen = await client.recv_until_id(detach_id, timeout=5)
        if "error" in detach_response:
            raise SmokeError(
                "Target.detachFromTarget failed with IO interrupts in flight: "
                f"{detach_response}"
            )
        detach_completed = True
    finally:
        if not detach_completed:
            await _best_effort_terminate_execution(
                client,
                page.primary_session_id,
            )

    termination_confirmed = False
    try:
        terminate_id = await client.send(
            "Runtime.terminateExecution",
            session_id=page.primary_session_id,
        )
        completed, termination_seen = await _recv_responses(
            client,
            {run_id, terminate_id},
            timeout=10,
        )
        if "error" in completed[terminate_id]:
            raise SmokeError(
                "primary session could not terminate JavaScript after auxiliary detach: "
                f"{completed[terminate_id]}"
            )
        termination_confirmed = True
    finally:
        if not termination_confirmed:
            await _best_effort_terminate_execution(
                client,
                page.primary_session_id,
            )
    seen.extend(termination_seen)

    replacement_session_id = await _attach(client, page.target_id)
    for method in ("Runtime.enable", "Debugger.enable"):
        message_id = await client.send(method, session_id=replacement_session_id)
        response, enabled_seen = await client.recv_until_id(message_id, timeout=5)
        seen.extend(enabled_seen)
        if "error" in response:
            raise SmokeError(
                f"replacement session could not enable {method}: {response}"
            )
    recovery_id = await client.send(
        "Runtime.evaluate",
        {"expression": "6 * 7", "returnByValue": True},
        session_id=replacement_session_id,
    )
    recovery, recovery_seen = await client.recv_until_id(recovery_id, timeout=5)
    seen.extend(recovery_seen)
    assert_equal(
        recovery.get("result", {}).get("result", {}).get("value"),
        42,
        "replacement Inspector session after detach/interrupt race",
    )
    seen.extend(await _recv_for(client, 0.2))

    response_counts = {
        message_id: sum(message.get("id") == message_id for message in seen)
        for message_id in interrupt_ids
    }
    duplicates = {
        message_id: count for message_id, count in response_counts.items() if count > 1
    }
    if duplicates:
        raise SmokeError(
            "detached session produced duplicate responses for claimed IO commands: "
            f"{duplicates}"
        )
    settled_count = sum(count == 1 for count in response_counts.values())
    record_contract(
        results,
        "raw_cdp_session_detach_with_interrupts_in_flight",
        contract=(
            "Detaching a session while many IO interrupts are queued cannot double-claim a "
            "command or retain a destroyed V8InspectorSession. Another session terminates "
            "the active script, and a fresh attachment remains usable."
        ),
        source="Chromium DevToolsSession detach/IO executable race probe",
        commands=[
            "Runtime.runScript",
            "Debugger.getScriptSource x32",
            "Target.detachFromTarget",
            "Runtime.terminateExecution",
            "Target.attachToTarget",
            "Runtime.evaluate",
        ],
        observed={
            "issuedInterrupts": len(interrupt_ids),
            "settledBeforeOrDuringDetach": settled_count,
            "droppedWithDetachedSession": len(interrupt_ids) - settled_count,
        },
    )


async def _context_dispose_with_interrupts_in_flight(
    client: RawCdpClient,
    page: InspectorRoutingPage,
    fixture: str,
    results: list[dict[str, Any]],
) -> None:
    await _reset_witness(fixture)
    source = """const xhr = new XMLHttpRequest();
xhr.open('GET', '/inspector-routing-witness/entered', false);
xhr.send();
for (;;) {}"""
    compile_id = await client.send(
        "Runtime.compileScript",
        {
            "expression": source,
            "sourceURL": "inspector-routing-context-dispose.js",
            "persistScript": True,
        },
        session_id=page.primary_session_id,
    )
    compiled, _ = await client.recv_until_id(compile_id, timeout=5)
    script_id = compiled.get("result", {}).get("scriptId")
    if not isinstance(script_id, str) or not script_id:
        raise SmokeError(f"context-dispose probe compiled no script: {compiled}")

    run_id = await client.send(
        "Runtime.runScript",
        {"scriptId": script_id},
        session_id=page.primary_session_id,
    )
    await _wait_for_witness(fixture, expected_count=1)

    interrupt_ids = [
        await client.send(
            "Debugger.getScriptSource",
            {"scriptId": script_id},
            session_id=page.auxiliary_session_id,
        )
        for _ in range(64)
    ]
    dispose_confirmed = False
    try:
        dispose_id = await client.send(
            "Target.disposeBrowserContext",
            {"browserContextId": page.browser_context_id},
        )
        dispose_response, seen = await client.recv_until_id(dispose_id, timeout=10)
        if "error" in dispose_response:
            raise SmokeError(
                "Target.disposeBrowserContext failed with IO interrupts in flight: "
                f"{dispose_response}"
            )
        dispose_confirmed = True
    finally:
        if not dispose_confirmed:
            await _best_effort_terminate_execution(
                client,
                page.primary_session_id,
            )

    seen.extend(await _recv_for(client, 0.2))
    response_counts = {
        message_id: sum(message.get("id") == message_id for message in seen)
        for message_id in interrupt_ids
    }
    duplicates = {
        message_id: count for message_id, count in response_counts.items() if count > 1
    }
    if duplicates:
        raise SmokeError(
            "context teardown produced duplicate responses for IO commands: "
            f"{duplicates}"
        )
    busy_response_count = sum(message.get("id") == run_id for message in seen)
    if busy_response_count > 1:
        raise SmokeError(
            "context teardown responded more than once to active JavaScript: "
            f"count={busy_response_count}"
        )

    stale_session_id = await client.send(
        "Runtime.evaluate",
        {"expression": "1"},
        session_id=page.auxiliary_session_id,
    )
    stale_responses, stale_seen = await _recv_responses(
        client,
        {stale_session_id},
        timeout=5,
    )
    stale_session = stale_responses[stale_session_id]
    seen.extend(stale_seen)
    if "error" not in stale_session:
        raise SmokeError(
            "disposed BrowserContext retained an executable Inspector session: "
            f"{stale_session}"
        )

    targets_id = await client.send("Target.getTargets")
    targets, targets_seen = await client.recv_until_id(targets_id, timeout=5)
    seen.extend(targets_seen)
    target_infos = targets.get("result", {}).get("targetInfos")
    if not isinstance(target_infos, list):
        raise SmokeError(f"Target.getTargets returned no targetInfos: {targets}")
    if any(
        isinstance(target, dict) and target.get("targetId") == page.target_id
        for target in target_infos
    ):
        raise SmokeError(
            "disposed BrowserContext retained its target in Target.getTargets: "
            f"{page.target_id}"
        )

    version_id = await client.send("Browser.getVersion")
    version, version_seen = await client.recv_until_id(version_id, timeout=5)
    seen.extend(version_seen)
    product = version.get("result", {}).get("product")
    if not isinstance(product, str) or not product:
        raise SmokeError(f"browser connection died during context teardown: {version}")

    replacement = await _create_page(client, f"{fixture}/plain?context-recovery=1")
    try:
        recovery_id = await client.send(
            "Runtime.evaluate",
            {"expression": "6 * 7", "returnByValue": True},
            session_id=replacement.primary_session_id,
        )
        recovery, recovery_seen = await client.recv_until_id(recovery_id, timeout=5)
        seen.extend(recovery_seen)
        assert_equal(
            recovery.get("result", {}).get("result", {}).get("value"),
            42,
            "replacement context after active-JS/IO teardown",
        )
    finally:
        replacement_dispose_id = await client.send(
            "Target.disposeBrowserContext",
            {"browserContextId": replacement.browser_context_id},
        )
        await client.recv_until_id(replacement_dispose_id, timeout=5)

    settled_count = sum(count == 1 for count in response_counts.values())
    record_contract(
        results,
        "raw_cdp_context_dispose_with_interrupts_in_flight",
        contract=(
            "Disposing a BrowserContext tears down a target whose JavaScript is non-yielding "
            "while IO interrupts are in flight. No command is double-completed, the old "
            "session and target disappear, the browser connection survives, and a replacement "
            "context can execute JavaScript."
        ),
        source="Chromium BrowserContext/renderer teardown executable race probe",
        commands=[
            "Runtime.runScript",
            "Debugger.getScriptSource x64",
            "Target.disposeBrowserContext",
            "Runtime.evaluate (detached session rejection)",
            "Target.getTargets",
            "Browser.getVersion",
            "Target.createBrowserContext",
            "Runtime.evaluate",
        ],
        observed={
            "issuedInterrupts": len(interrupt_ids),
            "settledBeforeOrDuringDispose": settled_count,
            "droppedWithContext": len(interrupt_ids) - settled_count,
            "busyResponseCount": busy_response_count,
            "staleSessionError": stale_session["error"],
            "remainingTargetCount": len(target_infos),
            "browserProduct": product,
        },
    )


async def _page_crash_io_during_active_javascript(
    client: RawCdpClient,
    fixture: str,
    results: list[dict[str, Any]],
) -> None:
    discover_id = await client.send(
        "Target.setDiscoverTargets",
        {"discover": True},
    )
    await client.recv_until_id(discover_id, timeout=5)
    crash_page = await _create_page(client, f"{fixture}/plain?crash-probe=1")
    try:
        await _reset_witness(fixture)
        busy_id = await client.send(
            "Runtime.evaluate",
            {
                "expression": """(() => {
                  const xhr = new XMLHttpRequest();
                  xhr.open('GET', '/inspector-routing-witness/entered', false);
                  xhr.send();
                  for (;;) {}
                })()""",
                "returnByValue": True,
            },
            session_id=crash_page.primary_session_id,
        )
        await _wait_for_witness(fixture, expected_count=1)
        crash_id = await client.send(
            "Page.crash",
            session_id=crash_page.auxiliary_session_id,
        )

        seen: list[dict[str, Any]] = []
        target_crashed: dict[str, Any] | None = None
        deadline = asyncio.get_running_loop().time() + 5.0
        while target_crashed is None:
            remaining = deadline - asyncio.get_running_loop().time()
            if remaining <= 0:
                raise SmokeError(
                    "Page.crash did not cross the IO boundary during active JavaScript; "
                    f"seen={seen[-30:]}"
                )
            try:
                message = await asyncio.wait_for(client.recv(), timeout=remaining)
            except TimeoutError as error:
                raise SmokeError(
                    "Page.crash did not cross the IO boundary during active JavaScript; "
                    f"seen={seen[-30:]}"
                ) from error
            seen.append(message)
            if (
                message.get("method") == "Target.targetCrashed"
                and message.get("params", {}).get("targetId") == crash_page.target_id
            ):
                target_crashed = message
        seen.extend(await _recv_for(client, 0.2))
        crash_responses = [message for message in seen if message.get("id") == crash_id]
        if len(crash_responses) > 1:
            raise SmokeError(f"Page.crash responded more than once: {crash_responses}")
        busy_responses = [message for message in seen if message.get("id") == busy_id]
        if len(busy_responses) > 1:
            raise SmokeError(
                f"active JavaScript received duplicate crash teardown responses: {busy_responses}"
            )

        version_id = await client.send("Browser.getVersion")
        version, _ = await client.recv_until_id(version_id, timeout=5)
        product = version.get("result", {}).get("product")
        if not isinstance(product, str) or not product:
            raise SmokeError(f"browser connection died with crashed target: {version}")
        record_contract(
            results,
            "raw_cdp_page_crash_io_during_active_javascript",
            contract=(
                "Page.crash reaches the renderer through Chromium's non-V8 IO boundary while "
                "JavaScript is non-yielding, tears the target down once, and leaves the browser "
                "DevTools connection alive."
            ),
            source="Chromium DevToolsSession::ShouldSendOnIO executable probe",
            commands=["Runtime.evaluate", "Page.crash", "Browser.getVersion"],
            observed={
                "crashResponseCount": len(crash_responses),
                "busyResponseCount": len(busy_responses),
                "browserProduct": product,
            },
        )
    finally:
        try:
            dispose_id = await client.send(
                "Target.disposeBrowserContext",
                {"browserContextId": crash_page.browser_context_id},
            )
            await client.recv_until_id(dispose_id, timeout=5)
        except Exception:
            pass


async def _create_page(client: RawCdpClient, url: str) -> InspectorRoutingPage:
    context_id = await client.send("Target.createBrowserContext")
    context, _ = await client.recv_until_id(context_id, timeout=5)
    browser_context_id = context.get("result", {}).get("browserContextId")
    if not isinstance(browser_context_id, str) or not browser_context_id:
        raise SmokeError(f"Target.createBrowserContext returned no id: {context}")

    create_id = await client.send(
        "Target.createTarget",
        {"browserContextId": browser_context_id, "url": "about:blank"},
    )
    created, _ = await client.recv_until_id(create_id, timeout=5)
    target_id = created.get("result", {}).get("targetId")
    if not isinstance(target_id, str) or not target_id:
        raise SmokeError(f"Target.createTarget returned no id: {created}")

    primary_session_id = await _attach(client, target_id)
    for method in ("Runtime.enable", "Page.enable", "Debugger.enable"):
        message_id = await client.send(method, session_id=primary_session_id)
        await client.recv_until_id(message_id, timeout=5)
    navigate_id = await client.send(
        "Page.navigate",
        {"url": url},
        session_id=primary_session_id,
    )
    saw_response = False
    saw_load = False
    seen: list[dict[str, Any]] = []
    deadline = asyncio.get_running_loop().time() + 10.0
    while not (saw_response and saw_load):
        remaining = deadline - asyncio.get_running_loop().time()
        if remaining <= 0:
            raise SmokeError(f"timed out navigating Inspector routing page: {seen[-20:]}")
        try:
            message = await asyncio.wait_for(client.recv(), timeout=remaining)
        except TimeoutError as error:
            raise SmokeError(
                f"timed out navigating Inspector routing page: {seen[-20:]}"
            ) from error
        seen.append(message)
        if message.get("id") == navigate_id:
            if "error" in message:
                raise SmokeError(f"Page.navigate failed: {message}")
            saw_response = True
        if (
            message.get("sessionId") == primary_session_id
            and message.get("method") == "Page.loadEventFired"
        ):
            saw_load = True

    auxiliary_session_id = await _attach(client, target_id)
    for method in ("Runtime.enable", "Debugger.enable"):
        message_id = await client.send(method, session_id=auxiliary_session_id)
        await client.recv_until_id(message_id, timeout=5)
    return InspectorRoutingPage(
        browser_context_id=browser_context_id,
        target_id=target_id,
        primary_session_id=primary_session_id,
        auxiliary_session_id=auxiliary_session_id,
    )


async def _attach(client: RawCdpClient, target_id: str) -> str:
    attach_id = await client.send(
        "Target.attachToTarget",
        {"targetId": target_id, "flatten": True},
    )
    attached, _ = await client.recv_until_id(attach_id, timeout=5)
    session_id = attached.get("result", {}).get("sessionId")
    if not isinstance(session_id, str) or not session_id:
        raise SmokeError(f"Target.attachToTarget returned no sessionId: {attached}")
    return session_id


async def _recv_paused_for_both_sessions(
    client: RawCdpClient,
    page: InspectorRoutingPage,
    paused_command_id: int,
) -> dict[str, dict[str, Any]]:
    paused_events: dict[str, dict[str, Any]] = {}
    seen: list[dict[str, Any]] = []
    expected_sessions = {page.primary_session_id, page.auxiliary_session_id}
    deadline = asyncio.get_running_loop().time() + 5.0
    while paused_events.keys() != expected_sessions:
        remaining = deadline - asyncio.get_running_loop().time()
        if remaining <= 0:
            raise SmokeError(
                "timed out waiting for both Debugger.paused events; "
                f"sessions={paused_events.keys()} seen={seen[-20:]}"
            )
        try:
            message = await asyncio.wait_for(client.recv(), timeout=remaining)
        except TimeoutError as error:
            raise SmokeError(
                "timed out waiting for both Debugger.paused events; "
                f"sessions={paused_events.keys()} seen={seen[-20:]}"
            ) from error
        seen.append(message)
        if message.get("id") == paused_command_id:
            raise SmokeError("debugger evaluation completed before Debugger.paused")
        session_id = message.get("sessionId")
        if (
            session_id in expected_sessions
            and message.get("method") == "Debugger.paused"
        ):
            paused_events[session_id] = message
    return paused_events


async def _recv_responses(
    client: RawCdpClient,
    expected_ids: set[int],
    *,
    timeout: float,
) -> tuple[dict[int, dict[str, Any]], list[dict[str, Any]]]:
    responses: dict[int, dict[str, Any]] = {}
    seen: list[dict[str, Any]] = []
    deadline = asyncio.get_running_loop().time() + timeout
    while responses.keys() != expected_ids:
        remaining = deadline - asyncio.get_running_loop().time()
        if remaining <= 0:
            raise SmokeError(
                "timed out waiting for Inspector routing responses; "
                f"expected={sorted(expected_ids)} responses={responses} seen={seen[-20:]}"
            )
        try:
            message = await asyncio.wait_for(client.recv(), timeout=remaining)
        except TimeoutError as error:
            raise SmokeError(
                "timed out waiting for Inspector routing responses; "
                f"expected={sorted(expected_ids)} responses={responses} seen={seen[-20:]}"
            ) from error
        seen.append(message)
        message_id = message.get("id")
        if message_id in expected_ids:
            if message_id in responses:
                raise SmokeError(f"duplicate CDP response id={message_id}: {seen[-20:]}")
            responses[message_id] = message
    return responses, seen


async def _recv_for(client: RawCdpClient, duration: float) -> list[dict[str, Any]]:
    seen: list[dict[str, Any]] = []
    deadline = asyncio.get_running_loop().time() + duration
    while True:
        remaining = deadline - asyncio.get_running_loop().time()
        if remaining <= 0:
            return seen
        try:
            seen.append(await asyncio.wait_for(client.recv(), timeout=remaining))
        except TimeoutError:
            return seen


async def _recv_until_session_event(
    client: RawCdpClient,
    session_id: str,
    method: str,
    *,
    timeout: float,
) -> dict[str, Any]:
    seen: list[dict[str, Any]] = []
    deadline = asyncio.get_running_loop().time() + timeout
    while True:
        remaining = deadline - asyncio.get_running_loop().time()
        if remaining <= 0:
            raise SmokeError(
                f"timed out waiting for {session_id} {method}; seen={seen[-20:]}"
            )
        message = await asyncio.wait_for(client.recv(), timeout=remaining)
        seen.append(message)
        if message.get("sessionId") == session_id and message.get("method") == method:
            return message


async def _close_raw_cdp_client(client: RawCdpClient) -> None:
    try:
        await asyncio.wait_for(client.websocket.close(), timeout=1)
    except Exception:
        transport = getattr(client.websocket, "transport", None)
        if transport is not None:
            transport.abort()


async def _best_effort_terminate_execution(
    client: RawCdpClient,
    session_id: str,
) -> None:
    try:
        terminate_id = await client.send(
            "Runtime.terminateExecution",
            session_id=session_id,
        )
        await client.recv_until_id(terminate_id, timeout=2)
    except Exception:
        pass


def _read_json(url: str) -> dict[str, Any]:
    with urllib.request.urlopen(url, timeout=2) as response:
        payload = json.loads(response.read().decode("utf-8"))
    if not isinstance(payload, dict):
        raise SmokeError(f"fixture returned non-object JSON: {payload!r}")
    return payload


async def _reset_witness(fixture: str) -> None:
    await asyncio.to_thread(
        _read_json,
        f"{fixture}/inspector-routing-witness/reset",
    )


async def _wait_for_witness(fixture: str, *, expected_count: int) -> None:
    deadline = asyncio.get_running_loop().time() + 5.0
    while True:
        status = await asyncio.to_thread(
            _read_json,
            f"{fixture}/inspector-routing-witness/status",
        )
        if status.get("enteredCount") == expected_count:
            return
        if asyncio.get_running_loop().time() >= deadline:
            raise SmokeError(
                f"timed out waiting for active-JS witness count {expected_count}: {status}"
            )
        await asyncio.sleep(0.01)
