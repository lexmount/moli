from __future__ import annotations

import asyncio
import json
from pathlib import Path
from tempfile import TemporaryDirectory
from typing import Any, Awaitable, Callable
from urllib.parse import urlsplit

import websockets

from ..assertions import assert_equal, assert_true, record
from ..scenarios import record_failure


BidiScenario = Callable[[Any, str, str, list[dict[str, Any]]], Awaitable[None]]
CAPTURE_SCREENSHOT_UNSUPPORTED_MESSAGE = (
    "Page.captureScreenshot is not supported: renderer screenshots are not implemented."
)


async def run_bidi_group(
    endpoint: str,
    fixture: str,
    results: list[dict[str, Any]],
    continue_on_failure: bool = False,
) -> None:
    if continue_on_failure:
        for name, scenario in _bidi_scenarios():
            try:
                await _run_bidi_isolated_scenario(endpoint, fixture, results, scenario)
            except Exception as error:
                record_failure(results, "bidi", name, error)
        return

    ws_endpoint = endpoint.replace("http://", "ws://", 1).rstrip("/") + "/session"
    async with websockets.connect(ws_endpoint, max_size=2**24) as websocket:
        context = await _bootstrap_bidi_context(websocket, fixture, results)

        scenario_error: BaseException | None = None
        try:
            for _name, scenario in _bidi_scenarios():
                await scenario(websocket, context, fixture, results)
        except Exception as error:
            scenario_error = error
        finally:
            try:
                await _end_bidi_session(websocket, results)
            except Exception as error:
                if scenario_error is None:
                    scenario_error = error
        if scenario_error is not None:
            raise scenario_error


def _bidi_scenarios() -> tuple[tuple[str, BidiScenario], ...]:
    return (
        ("bidi_input_actions", _run_bidi_input_smoke),
        ("bidi_network_get_data", _run_bidi_network_get_data_smoke),
        ("bidi_network_set_cache_behavior", _run_bidi_network_cache_behavior_smoke),
        ("bidi_shared_worker", _run_bidi_shared_worker_smoke),
        ("bidi_profile_emulation_storage", _run_bidi_profile_emulation_storage_smoke),
        ("bidi_download", _run_bidi_download_smoke),
    )


async def _run_bidi_isolated_scenario(
    endpoint: str,
    fixture: str,
    results: list[dict[str, Any]],
    scenario: BidiScenario,
) -> None:
    ws_endpoint = endpoint.replace("http://", "ws://", 1).rstrip("/") + "/session"
    async with websockets.connect(ws_endpoint, max_size=2**24) as websocket:
        context = await _bootstrap_bidi_context(websocket, fixture, results)
        scenario_error: BaseException | None = None
        try:
            await scenario(websocket, context, fixture, results)
        except Exception as error:
            scenario_error = error
        finally:
            try:
                await _end_bidi_session(websocket, results)
            except Exception as error:
                if scenario_error is None:
                    scenario_error = error
        if scenario_error is not None:
            raise scenario_error


async def _bootstrap_bidi_context(
    websocket: Any,
    fixture: str,
    results: list[dict[str, Any]],
) -> str:
    await _send(websocket, 1, "session.status", {})
    status = await _recv(websocket)
    assert_equal(status["type"], "success", "BiDi session.status type")
    assert_equal(status["result"]["ready"], True, "BiDi session.status ready")

    await _send(websocket, 2, "session.new", {"capabilities": {}})
    session = await _recv(websocket)
    assert_equal(session["type"], "success", "BiDi session.new type")
    session_id = session["result"]["sessionId"]
    assert_true(isinstance(session_id, str) and session_id, "BiDi session id should be non-empty")
    record(results, "bidi_session_new", {"sessionId": session_id})

    await _send(websocket, 3, "browsingContext.create", {"type": "tab"})
    create = await _recv(websocket)
    assert_equal(create["type"], "success", "BiDi browsingContext.create type")
    context = create["result"]["context"]
    assert_true(isinstance(context, str) and context, "BiDi context id should be non-empty")

    await _send(
        websocket,
        4,
        "session.subscribe",
        {
            "events": ["browsingContext.domContentLoaded"],
            "contexts": [context],
        },
    )
    subscribe = await _recv(websocket)
    assert_equal(subscribe["type"], "success", "BiDi session.subscribe type")

    page_url = f"{fixture}/webdriver/basic"
    await _send(
        websocket,
        5,
        "browsingContext.navigate",
        {"context": context, "url": page_url, "wait": "complete"},
    )
    messages = await _recv_until_id(websocket, 5)
    navigate = messages[-1]
    assert_equal(navigate["type"], "success", "BiDi browsingContext.navigate type")
    assert_equal(navigate["result"]["url"], page_url, "BiDi navigate URL")
    lifecycle = await _collect_lifecycle_events(websocket, messages, context)
    assert_equal(
        [event["method"] for event in lifecycle],
        ["browsingContext.domContentLoaded"],
        "BiDi lifecycle event order",
    )
    record(results, "bidi_browsing_context_navigation", {"context": context})
    return context


async def _end_bidi_session(websocket: Any, results: list[dict[str, Any]]) -> None:
    await _send(websocket, 80, "session.end", {})
    end = await asyncio.wait_for(_recv_until_id(websocket, 80), timeout=5)
    assert_equal(end[-1]["type"], "success", "BiDi session.end type")
    assert_equal(end[-1]["result"], {}, "BiDi session.end result")
    record(results, "bidi_session_end")


async def _send(websocket: Any, id_: int, method: str, params: dict[str, Any]) -> None:
    await websocket.send(json.dumps({"id": id_, "method": method, "params": params}, separators=(",", ":")))


async def _run_bidi_input_smoke(
    websocket: Any,
    context: str,
    fixture: str,
    results: list[dict[str, Any]],
) -> None:
    await _send(
        websocket,
        6,
        "script.evaluate",
        {
            "expression": (
                "(() => {"
                "const field = document.querySelector('#field');"
                "window.__events = [];"
                "field.value = '';"
                "field.focus();"
                "field.addEventListener('keydown', event => window.__events.push(event.type + ':' + event.key));"
                "field.addEventListener('keyup', event => window.__events.push(event.type + ':' + event.key));"
                "return 'ready';"
                "})()"
            ),
            "target": {"context": context},
            "awaitPromise": False,
        },
    )
    setup = (await _recv_until_id(websocket, 6))[-1]
    assert_equal(setup["type"], "success", "BiDi script.evaluate input setup type")
    assert_equal(setup["result"]["result"]["value"], "ready", "BiDi input setup result")

    await _send(
        websocket,
        7,
        "input.performActions",
        {
            "context": context,
            "actions": [
                {
                    "type": "key",
                    "id": "keyboard",
                    "actions": [
                        {"type": "keyDown", "value": "a"},
                        {"type": "keyUp", "value": "a"},
                    ],
                }
            ],
        },
    )
    type_a = (await _recv_until_id(websocket, 7))[-1]
    assert_equal(type_a["type"], "success", "BiDi input.performActions key type")
    assert_equal(type_a["result"], {}, "BiDi input.performActions result")

    await _send(
        websocket,
        8,
        "input.performActions",
        {
            "context": context,
            "actions": [
                {
                    "type": "key",
                    "id": "keyboard",
                    "actions": [
                        {"type": "keyDown", "value": "b"},
                    ],
                }
            ],
        },
    )
    hold_b = (await _recv_until_id(websocket, 8))[-1]
    assert_equal(hold_b["type"], "success", "BiDi input.performActions held key type")
    assert_equal(hold_b["result"], {}, "BiDi input.performActions held key result")

    await _send(websocket, 9, "input.releaseActions", {"context": context})
    release = (await _recv_until_id(websocket, 9))[-1]
    assert_equal(release["type"], "success", "BiDi input.releaseActions type")
    assert_equal(release["result"], {}, "BiDi input.releaseActions result")

    await _send(
        websocket,
        10,
        "script.evaluate",
        {
            "expression": "window.__events.join(',')",
            "target": {"context": context},
            "awaitPromise": False,
        },
    )
    summary = (await _recv_until_id(websocket, 10))[-1]
    assert_equal(summary["type"], "success", "BiDi input summary type")
    assert_equal(
        summary["result"]["result"]["value"],
        "keydown:a,keyup:a,keydown:b,keyup:b",
        "BiDi input key event order",
    )
    record(results, "bidi_input_actions", {"context": context})

    await _send(
        websocket,
        11,
        "script.evaluate",
        {
            "expression": (
                "(() => {"
                "window.__bidiOriginClicked = false;"
                "window.__bidiOriginWheel = null;"
                "const button = document.getElementById('clicker');"
                "button.onclick = () => { window.__bidiOriginClicked = true; };"
                "const wheel = document.createElement('div');"
                "wheel.id = 'bidi-origin-wheel';"
                "wheel.style.position = 'fixed';"
                "wheel.style.left = '20px';"
                "wheel.style.top = '80px';"
                "wheel.style.width = '200px';"
                "wheel.style.height = '200px';"
                "wheel.textContent = 'wheel target';"
                "document.body.insertBefore(wheel, document.body.firstChild);"
                "document.addEventListener('wheel', event => {"
                "window.__bidiOriginWheel = {type: event.type, deltaX: event.deltaX, deltaY: event.deltaY};"
                "});"
                "return 'origin-ready';"
                "})()"
            ),
            "target": {"context": context},
            "awaitPromise": False,
        },
    )
    origin_setup = (await _recv_until_id(websocket, 11))[-1]
    assert_equal(origin_setup["type"], "success", "BiDi input origin setup type")
    assert_equal(origin_setup["result"]["result"]["value"], "origin-ready", "BiDi input origin setup result")

    await _send(
        websocket,
        12,
        "script.evaluate",
        {
            "expression": "document.getElementById('clicker')",
            "target": {"context": context},
            "awaitPromise": False,
        },
    )
    button = (await _recv_until_id(websocket, 12))[-1]
    button_shared_id = button["result"]["result"]["sharedId"]
    assert_true(isinstance(button_shared_id, str) and button_shared_id, "BiDi input button sharedId")

    await _send(
        websocket,
        13,
        "script.evaluate",
        {
            "expression": "document.getElementById('bidi-origin-wheel')",
            "target": {"context": context},
            "awaitPromise": False,
        },
    )
    wheel = (await _recv_until_id(websocket, 13))[-1]
    wheel_shared_id = wheel["result"]["result"]["sharedId"]
    assert_true(isinstance(wheel_shared_id, str) and wheel_shared_id, "BiDi input wheel sharedId")

    await _send(
        websocket,
        14,
        "input.performActions",
        {
            "context": context,
            "actions": [
                {
                    "type": "pointer",
                    "id": "origin-mouse",
                    "parameters": {"pointerType": "mouse"},
                    "actions": [
                        {
                            "type": "pointerMove",
                            "origin": {"type": "element", "sharedId": button_shared_id},
                            "x": 0,
                            "y": 0,
                        },
                        {"type": "pointerDown", "button": 0},
                        {"type": "pointerUp", "button": 0},
                    ],
                }
            ],
        },
    )
    click = (await _recv_until_id(websocket, 14))[-1]
    assert_equal(click["type"], "error", f"BiDi input element-origin click type response={click}")
    assert_equal(click["error"], "unsupported operation", "BiDi input element-origin click error")
    assert_true(
        "Input.dispatchMouseEvent" in click["message"] and "layout hit testing" in click["message"],
        f"BiDi input element-origin click unsupported message: {click}",
    )

    await _send(
        websocket,
        15,
        "input.performActions",
        {
            "context": context,
            "actions": [
                {
                    "type": "wheel",
                    "id": "origin-wheel",
                    "actions": [
                        {
                            "type": "scroll",
                            "origin": {"type": "element", "sharedId": wheel_shared_id},
                            "x": 1,
                            "y": 2,
                            "deltaX": 7,
                            "deltaY": 13,
                        }
                    ],
                }
            ],
        },
    )
    scroll = (await _recv_until_id(websocket, 15))[-1]
    assert_equal(scroll["type"], "error", f"BiDi input element-origin wheel type response={scroll}")
    assert_equal(scroll["error"], "unsupported operation", "BiDi input element-origin wheel error")
    assert_true(
        "Input.dispatchMouseEvent" in scroll["message"] and "layout hit testing" in scroll["message"],
        f"BiDi input element-origin wheel unsupported message: {scroll}",
    )

    await _send(
        websocket,
        16,
        "script.evaluate",
        {
            "expression": "JSON.stringify({clicked: window.__bidiOriginClicked, wheel: window.__bidiOriginWheel})",
            "target": {"context": context},
            "awaitPromise": False,
        },
    )
    boundary_state = (await _recv_until_id(websocket, 16))[-1]
    assert_equal(boundary_state["type"], "success", "BiDi coordinate boundary state type")
    assert_equal(
        json.loads(boundary_state["result"]["result"]["value"]),
        {"clicked": False, "wheel": None},
        "BiDi unsupported coordinate actions do not dispatch DOM events",
    )

    record(results, "bidi_input_element_origin_coordinate_boundary", {"context": context})

    with TemporaryDirectory(prefix="moli-webdriver-smoke-") as directory:
        first_file = Path(directory) / "bidi-upload-a.txt"
        second_file = Path(directory) / "bidi-upload-b.txt"
        first_file.write_bytes(b"alpha")
        second_file.write_bytes(b"bravo!")

        await _send(
            websocket,
            17,
            "script.evaluate",
            {
                "expression": (
                    "(() => {"
                    "window.__bidiFileEvents = [];"
                    "const input = document.createElement('input');"
                    "input.id = 'bidi-upload';"
                    "input.type = 'file';"
                    "input.multiple = true;"
                    "input.addEventListener('input', () => window.__bidiFileEvents.push('input:' + input.files.length));"
                    "input.addEventListener('change', () => window.__bidiFileEvents.push('change:' + input.files.length));"
                    "document.body.appendChild(input);"
                    "return input;"
                    "})()"
                ),
                "target": {"context": context},
                "awaitPromise": False,
            },
        )
        file_input = (await _recv_until_id(websocket, 17))[-1]
        assert_equal(file_input["type"], "success", "BiDi input.setFiles setup type")
        file_shared_id = file_input["result"]["result"]["sharedId"]
        assert_true(isinstance(file_shared_id, str) and file_shared_id, "BiDi file input sharedId")

        await _send(
            websocket,
            18,
            "input.setFiles",
            {
                "context": context,
                "element": {"sharedId": file_shared_id},
                "files": [str(first_file), str(second_file)],
            },
        )
        set_files = (await _recv_until_id(websocket, 18))[-1]
        assert_equal(set_files["type"], "success", "BiDi input.setFiles type")
        assert_equal(set_files["result"], {}, "BiDi input.setFiles result")

        await _send(
            websocket,
            19,
            "script.evaluate",
            {
                "expression": (
                    "(() => {"
                    "const input = document.getElementById('bidi-upload');"
                    "return JSON.stringify({"
                    "length: input.files.length,"
                    "names: Array.from(input.files).map(file => file.name).join('|'),"
                    "sizes: Array.from(input.files).map(file => file.size).join('|'),"
                    "value: input.value,"
                    "events: window.__bidiFileEvents.join(',')"
                    "});"
                    "})()"
                ),
                "target": {"context": context},
                "awaitPromise": False,
            },
        )
        file_summary = (await _recv_until_id(websocket, 19))[-1]
        assert_equal(file_summary["type"], "success", "BiDi input.setFiles summary type")
        summary = json.loads(file_summary["result"]["result"]["value"])
        assert_equal(summary["length"], 2, "BiDi input.setFiles FileList length")
        assert_equal(
            summary["names"],
            "bidi-upload-a.txt|bidi-upload-b.txt",
            "BiDi input.setFiles FileList names",
        )
        assert_equal(summary["sizes"], "5|6", "BiDi input.setFiles FileList sizes")
        assert_equal(
            summary["value"],
            "C:\\fakepath\\bidi-upload-a.txt",
            "BiDi input.setFiles value fake path",
        )
        assert_equal(
            summary["events"],
            "input:2,change:2",
            "BiDi input.setFiles event order",
        )
        record(results, "bidi_input_set_files", {"context": context})

    input_navigation_url = f"{fixture}/webdriver/input-navigation"
    input_navigation_destination = f"{fixture}/webdriver/input-navigation-complete"
    await _navigate_complete(
        websocket,
        20,
        context,
        input_navigation_url,
        "BiDi input-navigation setup",
    )
    focus = await _evaluate_remote_value(
        websocket,
        21,
        context,
        "document.getElementById('navigation-field').focus(); document.activeElement.id",
        "BiDi input-navigation focus",
    )
    assert_equal(focus["value"], "navigation-field", "BiDi input-navigation active element")

    await _send(
        websocket,
        22,
        "input.performActions",
        {
            "context": context,
            "actions": [
                {
                    "type": "key",
                    "id": "navigation-keyboard",
                    "actions": [{"type": "keyDown", "value": "\ue007"}],
                }
            ],
        },
    )
    action_messages = await asyncio.wait_for(_recv_until_id(websocket, 22), timeout=5)
    action = action_messages[-1]
    assert_equal(
        action["type"],
        "success",
        f"BiDi input action responds across Page replacement: {action!r}",
    )
    assert_equal(action["result"], {}, "BiDi input-navigation action result")

    lifecycle = _find_bidi_event(
        action_messages,
        "browsingContext.domContentLoaded",
        lambda message: message.get("params", {}).get("url") == input_navigation_destination,
    )
    if lifecycle is None:
        lifecycle = await _recv_until_bidi_event(
            websocket,
            "browsingContext.domContentLoaded",
            "BiDi input-navigation DOMContentLoaded",
            lambda message: message.get("params", {}).get("url") == input_navigation_destination,
        )
    assert_equal(
        lifecycle["params"]["context"],
        context,
        "BiDi input-navigation lifecycle context",
    )

    await _send(websocket, 23, "input.releaseActions", {"context": context})
    release = await _recv_success(websocket, 23, "BiDi input-navigation releaseActions")
    assert_equal(release["result"], {}, "BiDi input-navigation release result")
    marker = await _evaluate_remote_value(
        websocket,
        24,
        context,
        "document.getElementById('input-navigation-complete')?.textContent",
        "BiDi input-navigation replacement marker",
    )
    assert_equal(
        marker["value"],
        "input navigation complete",
        "BiDi input-navigation replacement Page remains usable",
    )
    record(results, "bidi_input_navigation_replacement", {"context": context})


async def _run_bidi_network_get_data_smoke(
    websocket: Any, context: str, fixture: str, results: list[dict[str, Any]]
) -> None:
    await _send(
        websocket,
        20,
        "session.subscribe",
        {
            "events": ["network.responseCompleted"],
            "contexts": [context],
        },
    )
    subscribe = (await _recv_until_id(websocket, 20))[-1]
    assert_equal(subscribe["type"], "success", "BiDi network subscribe type")

    await _send(
        websocket,
        21,
        "network.addDataCollector",
        {
            "dataTypes": ["request", "response"],
            "maxEncodedDataSize": 1000,
            "contexts": [context],
        },
    )
    add_collector = await _recv_success(websocket, 21, "BiDi network.addDataCollector")
    collector = add_collector["result"]["collector"]
    assert_true(isinstance(collector, str) and collector, "BiDi network data collector id")

    data_url = f"{fixture}/webdriver/network-data"
    await _send(
        websocket,
        22,
        "script.evaluate",
        {
            "expression": (
                f"fetch({data_url!r}, "
                "{ method: 'POST', body: 'webdriver request body' }"
                ").then(response => response.text())"
            ),
            "target": {"context": context},
            "awaitPromise": True,
        },
    )
    messages = await _recv_until_id(websocket, 22)
    evaluate = messages[-1]
    assert_equal(evaluate["type"], "success", "BiDi network fetch evaluate type")
    assert_equal(
        evaluate["result"]["result"]["value"],
        "webdriver network body",
        "BiDi network fetch body",
    )

    response_completed = await _find_network_response_completed(websocket, messages, data_url)
    request_id = response_completed["params"]["request"]["request"]
    assert_true(isinstance(request_id, str) and request_id, "BiDi network request id")

    await _send(
        websocket,
        23,
        "network.getData",
        {
            "request": request_id,
            "dataType": "request",
            "collector": collector,
        },
    )
    request_data = (await _recv_until_id(websocket, 23))[-1]
    assert_equal(
        request_data["type"],
        "success",
        f"BiDi network.getData request type: {request_data!r}",
    )
    assert_equal(
        request_data["result"]["bytes"],
        {"type": "string", "value": "webdriver request body"},
        "BiDi network.getData request bytes",
    )

    await _send(
        websocket,
        24,
        "network.getData",
        {
            "request": request_id,
            "dataType": "response",
            "collector": collector,
        },
    )
    data = (await _recv_until_id(websocket, 24))[-1]
    assert_equal(data["type"], "success", f"BiDi network.getData type: {data!r}")
    assert_equal(
        data["result"]["bytes"],
        {"type": "string", "value": "webdriver network body"},
        "BiDi network.getData bytes",
    )
    record(results, "bidi_network_get_data", {"context": context, "request": request_id})


async def _run_bidi_network_cache_behavior_smoke(
    websocket: Any,
    context: str,
    fixture: str,
    results: list[dict[str, Any]],
) -> None:
    await _send(
        websocket,
        24,
        "network.setCacheBehavior",
        {
            "cacheBehavior": "bypass",
            "contexts": [context],
        },
    )
    scoped = await _recv_success(websocket, 24, "BiDi network.setCacheBehavior scoped bypass")
    assert_equal(scoped["result"], {}, "BiDi network.setCacheBehavior scoped result")

    await _send(
        websocket,
        25,
        "network.setCacheBehavior",
        {
            "cacheBehavior": "default",
        },
    )
    default = await _recv_success(websocket, 25, "BiDi network.setCacheBehavior global default")
    assert_equal(default["result"], {}, "BiDi network.setCacheBehavior global result")
    record(results, "bidi_network_set_cache_behavior", {"context": context})


async def _run_bidi_shared_worker_smoke(
    websocket: Any,
    context: str,
    fixture: str,
    results: list[dict[str, Any]],
) -> None:
    page_url = f"{fixture}/webdriver/shared-worker"
    worker_url = f"{fixture}/webdriver/shared-worker.js"
    await _navigate_complete(websocket, 26, context, page_url, "BiDi shared worker page navigation")

    await _send(
        websocket,
        27,
        "session.subscribe",
        {
            "events": [
                "browsingContext.contextCreated",
                "script.realmCreated",
            ],
        },
    )
    await _recv_success(websocket, 27, "BiDi shared worker event subscribe")

    await _send(
        websocket,
        28,
        "script.evaluate",
        {
            "expression": "globalThis.__webdriverSharedWorkerProbe('bidi').then(value => JSON.stringify(value))",
            "target": {"context": context},
            "awaitPromise": True,
        },
    )
    messages = await _recv_until_id(websocket, 28)
    evaluate = messages[-1]
    assert_equal(evaluate["type"], "success", f"BiDi shared worker evaluate type: {evaluate!r}")
    probe = json.loads(evaluate["result"]["result"]["value"])
    assert_equal(probe["kind"], "probe-result", "BiDi shared worker probe kind")
    assert_equal(probe["echoed"], "bidi", "BiDi shared worker probe echo")
    assert_equal(probe["name"], "webdriver-shared-worker-smoke", "BiDi shared worker name")
    assert_equal(probe["pathname"], "/webdriver/shared-worker.js", "BiDi shared worker pathname")
    assert_equal(probe["isSharedWorker"], True, "BiDi shared worker global scope")

    context_created = _find_bidi_event(
        messages,
        "browsingContext.contextCreated",
        lambda message: message.get("params", {}).get("url") == worker_url,
    )
    if context_created is None:
        context_created = await _recv_until_bidi_event(
            websocket,
            "browsingContext.contextCreated",
            "BiDi shared worker browsingContext.contextCreated",
            lambda message: message.get("params", {}).get("url") == worker_url,
        )
    shared_worker_context = context_created["params"]["context"]
    assert_true(isinstance(shared_worker_context, str) and shared_worker_context, "BiDi shared worker context id")
    assert_equal(context_created["params"]["clientWindow"], shared_worker_context, "BiDi shared worker clientWindow")
    assert_equal(context_created["params"]["children"], None, "BiDi shared worker children")

    realm_created = _find_bidi_event(
        messages,
        "script.realmCreated",
        lambda message: message.get("params", {}).get("type") == "shared-worker",
    )
    if realm_created is None:
        realm_created = await _recv_until_bidi_event(
            websocket,
            "script.realmCreated",
            "BiDi shared worker script.realmCreated",
            lambda message: message.get("params", {}).get("type") == "shared-worker",
        )
    assert_equal(realm_created["params"]["type"], "shared-worker", "BiDi shared worker realm type")
    assert_true(
        isinstance(realm_created["params"].get("realm"), str)
        and realm_created["params"]["realm"].startswith("shared-worker-"),
        f"BiDi shared worker realm id: {realm_created!r}",
    )

    await _send(
        websocket,
        29,
        "script.getRealms",
        {"context": shared_worker_context, "type": "shared-worker"},
    )
    realms = await _recv_success(websocket, 29, "BiDi shared worker script.getRealms")
    assert_true(
        any(realm.get("type") == "shared-worker" for realm in realms["result"]["realms"]),
        f"BiDi shared worker realms should include shared-worker realm: {realms!r}",
    )
    record(
        results,
        "bidi_shared_worker_context_and_realm",
        {
            "context": shared_worker_context,
            "realm": realm_created["params"]["realm"],
        },
    )


async def _run_bidi_profile_emulation_storage_smoke(
    websocket: Any,
    default_context: str,
    fixture: str,
    results: list[dict[str, Any]],
) -> None:
    fixture_host = urlsplit(fixture).hostname or "127.0.0.1"
    profile_url = f"{fixture}/webdriver/profile-echo"
    user_agent = "MoliWebDriverSmoke/1.0"
    locale = "fr-FR"
    timezone = "Asia/Tokyo"

    await _send(websocket, 40, "browser.createUserContext", {})
    create_user_context = await _recv_success(websocket, 40, "BiDi browser.createUserContext")
    user_context = create_user_context["result"]["userContext"]
    assert_true(isinstance(user_context, str) and user_context, "BiDi userContext id")

    await _send(
        websocket,
        41,
        "browsingContext.setViewport",
        {
            "userContexts": [user_context],
            "viewport": {"width": 360, "height": 240},
            "devicePixelRatio": 2.0,
        },
    )
    await _recv_success(websocket, 41, "BiDi userContext setViewport")

    await _send(
        websocket,
        42,
        "emulation.setUserAgentOverride",
        {"userContexts": [user_context], "userAgent": user_agent},
    )
    await _recv_success(websocket, 42, "BiDi userContext setUserAgentOverride")
    await _send(
        websocket,
        43,
        "emulation.setLocaleOverride",
        {"userContexts": [user_context], "locale": locale},
    )
    await _recv_success(websocket, 43, "BiDi userContext setLocaleOverride")
    await _send(
        websocket,
        44,
        "emulation.setTimezoneOverride",
        {"userContexts": [user_context], "timezone": timezone},
    )
    await _recv_success(websocket, 44, "BiDi userContext setTimezoneOverride")

    await _send(
        websocket,
        45,
        "browsingContext.create",
        {"type": "tab", "userContext": user_context},
    )
    create_context = await _recv_success(websocket, 45, "BiDi userContext browsingContext.create")
    custom_context = create_context["result"]["context"]
    assert_true(isinstance(custom_context, str) and custom_context, "BiDi custom context id")

    await _navigate_complete(websocket, 46, custom_context, profile_url, "BiDi custom context profile navigation")
    profile = await _evaluate_json_object(
        websocket,
        47,
        custom_context,
        (
            "JSON.stringify({"
            "userAgent:navigator.userAgent,"
            "language:navigator.language,"
            "languages:navigator.languages,"
            "locale:Intl.DateTimeFormat().resolvedOptions().locale,"
            "timeZone:Intl.DateTimeFormat().resolvedOptions().timeZone,"
            "width:innerWidth,"
            "height:innerHeight,"
            "dpr:devicePixelRatio,"
            "headerEcho:JSON.parse(document.getElementById('profile-echo').textContent)"
            "})"
        ),
        "BiDi profile runtime summary",
    )
    assert_equal(profile["userAgent"], user_agent, "BiDi userContext navigator.userAgent")
    assert_equal(profile["language"], locale, "BiDi userContext navigator.language")
    assert_equal(profile["languages"][0], locale, "BiDi userContext navigator.languages[0]")
    assert_equal(profile["locale"], locale, "BiDi userContext Intl locale")
    assert_equal(profile["timeZone"], timezone, "BiDi userContext timezone")
    assert_equal((profile["width"], profile["height"]), (360, 240), "BiDi userContext viewport size")
    assert_equal(profile["dpr"], 2, "BiDi userContext devicePixelRatio")
    assert_equal(profile["headerEcho"]["userAgent"], user_agent, "BiDi userContext User-Agent header")
    assert_equal(profile["headerEcho"]["acceptLanguage"], locale, "BiDi userContext Accept-Language header")

    online_default = await _evaluate_remote_value(
        websocket,
        140,
        custom_context,
        "navigator.onLine",
        "BiDi navigator.onLine default",
    )
    assert_equal(online_default["value"], True, "BiDi navigator.onLine default")
    await _send(
        websocket,
        141,
        "emulation.setNetworkConditions",
        {"userContexts": [user_context], "networkConditions": {"type": "offline"}},
    )
    await _recv_success(websocket, 141, "BiDi userContext setNetworkConditions offline")
    online_offline = await _evaluate_remote_value(
        websocket,
        142,
        custom_context,
        "navigator.onLine",
        "BiDi navigator.onLine offline",
    )
    assert_equal(online_offline["value"], False, "BiDi userContext network offline")
    await _send(
        websocket,
        143,
        "emulation.setNetworkConditions",
        {"userContexts": [user_context], "networkConditions": None},
    )
    await _recv_success(websocket, 143, "BiDi userContext setNetworkConditions reset")
    online_reset = await _evaluate_remote_value(
        websocket,
        144,
        custom_context,
        "navigator.onLine",
        "BiDi navigator.onLine reset",
    )
    assert_equal(online_reset["value"], True, "BiDi userContext network reset")

    await _send(
        websocket,
        48,
        "browsingContext.captureScreenshot",
        {
            "context": custom_context,
            "format": {"type": "image/png"},
        },
    )
    await _recv_error(
        websocket,
        48,
        "unsupported operation",
        CAPTURE_SCREENSHOT_UNSUPPORTED_MESSAGE,
        "BiDi captureScreenshot profile context",
    )
    record(results, "bidi_user_context_emulation", {"context": custom_context, "userContext": user_context})
    record(results, "bidi_viewport_screenshot_unsupported", {"context": custom_context})

    stored = await _evaluate_remote_value(
        websocket,
        49,
        custom_context,
        (
            "(() => {"
            "localStorage.setItem('bidi-local', 'custom-local');"
            "sessionStorage.setItem('bidi-session', 'custom-session');"
            "return 'stored';"
            "})()"
        ),
        "BiDi custom context storage setup",
    )
    assert_equal(stored["value"], "stored", "BiDi custom context storage setup result")
    await _navigate_complete(websocket, 50, custom_context, profile_url, "BiDi custom context profile reload")
    custom_storage = await _evaluate_json_object(
        websocket,
        51,
        custom_context,
        "JSON.stringify({local:localStorage.getItem('bidi-local'),session:sessionStorage.getItem('bidi-session')})",
        "BiDi custom context storage readback",
    )
    assert_equal(
        custom_storage,
        {"local": "custom-local", "session": "custom-session"},
        "BiDi custom context storage persists across navigation",
    )

    await _navigate_complete(websocket, 52, default_context, profile_url, "BiDi default context profile navigation")
    default_storage = await _evaluate_json_object(
        websocket,
        53,
        default_context,
        "JSON.stringify({local:localStorage.getItem('bidi-local'),session:sessionStorage.getItem('bidi-session')})",
        "BiDi default context storage isolation readback",
    )
    assert_equal(default_storage, {"local": None, "session": None}, "BiDi default context storage isolation")

    await _send(
        websocket,
        54,
        "storage.setCookie",
        {
            "cookie": {
                "name": "bidiSmoke",
                "value": {"type": "string", "value": "custom-cookie"},
                "domain": fixture_host,
                "path": "/",
                "sameSite": "lax",
            },
            "partition": {"type": "context", "context": custom_context},
        },
    )
    await _recv_success(websocket, 54, "BiDi storage.setCookie custom context")
    custom_cookies = await _get_bidi_cookies(websocket, 55, custom_context, fixture_host)
    assert_equal(
        [(cookie["name"], cookie["value"]["value"]) for cookie in custom_cookies],
        [("bidiSmoke", "custom-cookie")],
        "BiDi custom context cookie readback",
    )
    default_cookies = await _get_bidi_cookies(websocket, 56, default_context, fixture_host)
    assert_equal(default_cookies, [], "BiDi default context cookie isolation")

    await _navigate_complete(websocket, 57, custom_context, profile_url, "BiDi custom context cookie echo")
    cookie_echo = await _evaluate_json_object(
        websocket,
        58,
        custom_context,
        "document.getElementById('profile-echo').textContent",
        "BiDi custom context cookie echo body",
    )
    assert_true(
        "bidiSmoke=custom-cookie" in cookie_echo["cookie"],
        "BiDi custom context cookie should be sent on matching navigation",
    )
    record(results, "bidi_storage_cookie_isolation", {"context": custom_context})

    await _send(websocket, 59, "browser.removeUserContext", {"userContext": user_context})
    await _recv_success(websocket, 59, "BiDi browser.removeUserContext cleanup")


async def _run_bidi_download_smoke(
    websocket: Any,
    context: str,
    fixture: str,
    results: list[dict[str, Any]],
) -> None:
    download_page_url = f"{fixture}/webdriver/download-page"
    download_url = f"{fixture}/webdriver/download"

    with TemporaryDirectory(prefix="moli-webdriver-bidi-download-") as directory:
        await _send(
            websocket,
            60,
            "browser.setDownloadBehavior",
            {
                "downloadBehavior": {
                    "type": "allowed",
                    "destinationFolder": directory,
                }
            },
        )
        await _recv_success(websocket, 60, "BiDi browser.setDownloadBehavior allowed")
        await _send(
            websocket,
            61,
            "session.subscribe",
            {
                "events": [
                    "browsingContext.downloadWillBegin",
                    "browsingContext.downloadEnd",
                ],
                "contexts": [context],
            },
        )
        await _recv_success(websocket, 61, "BiDi download events subscribe")

        await _navigate_complete(websocket, 62, context, download_page_url, "BiDi download page navigation")
        click_messages = await _click_download_anchor_by_dom(
            websocket,
            63,
            context,
            "BiDi download click",
        )
        will_begin, download_end = await _collect_download_events(websocket, click_messages, download_url)

        assert_equal(will_begin["params"]["context"], context, "BiDi download willBegin context")
        assert_equal(will_begin["params"]["navigation"], None, "BiDi download willBegin navigation")
        assert_equal(will_begin["params"]["url"], download_url, "BiDi download willBegin URL")
        assert_equal(
            will_begin["params"]["suggestedFilename"],
            "smoke-download.txt",
            "BiDi download suggested filename",
        )
        assert_equal(download_end["params"]["context"], context, "BiDi downloadEnd context")
        assert_equal(download_end["params"]["navigation"], None, "BiDi downloadEnd navigation")
        assert_equal(download_end["params"]["status"], "complete", "BiDi downloadEnd status")
        assert_equal(download_end["params"]["url"], download_url, "BiDi downloadEnd URL")
        filepath = download_end["params"].get("filepath")
        assert_true(isinstance(filepath, str) and filepath, "BiDi downloadEnd filepath")
        assert_equal(Path(filepath).read_text(encoding="utf-8"), "download contents", "BiDi download artifact contents")
        record(results, "bidi_download_event_and_artifact", {"context": context})

        await _send(
            websocket,
            65,
            "browser.setDownloadBehavior",
            {"downloadBehavior": {"type": "denied"}},
        )
        await _recv_success(websocket, 65, "BiDi browser.setDownloadBehavior denied")
        await _navigate_complete(websocket, 66, context, download_page_url, "BiDi download denied page navigation")
        click_messages = await _click_download_anchor_by_dom(
            websocket,
            67,
            context,
            "BiDi denied download click",
        )
        denied_will_begin, denied_end = await _collect_download_events(websocket, click_messages, download_url)
        assert_equal(
            denied_will_begin["params"]["navigation"],
            None,
            "BiDi denied downloadWillBegin navigation",
        )
        assert_equal(denied_end["params"]["context"], context, "BiDi denied downloadEnd context")
        assert_equal(
            denied_end["params"]["navigation"],
            None,
            "BiDi denied downloadEnd navigation",
        )
        assert_equal(denied_end["params"]["status"], "canceled", "BiDi denied downloadEnd status")
        assert_true("filepath" not in denied_end["params"], "BiDi denied downloadEnd should not include filepath")
        record(results, "bidi_download_denied", {"context": context})


async def _recv_success(websocket: Any, id_: int, label: str) -> dict[str, Any]:
    response = (await _recv_until_id(websocket, id_))[-1]
    assert_equal(response["type"], "success", f"{label} type: {response!r}")
    return response


async def _recv_error(websocket: Any, id_: int, error: str, message: str, label: str) -> dict[str, Any]:
    response = (await _recv_until_id(websocket, id_))[-1]
    assert_equal(response["type"], "error", f"{label} type: {response!r}")
    assert_equal(response["error"], error, f"{label} error: {response!r}")
    assert_equal(response["message"], message, f"{label} message: {response!r}")
    return response


async def _click_download_anchor_by_dom(
    websocket: Any,
    id_: int,
    context: str,
    label: str,
) -> list[dict[str, Any]]:
    await _send(
        websocket,
        id_,
        "script.evaluate",
        {
            "expression": (
                "(() => {"
                "const anchor = document.getElementById('download');"
                "if (!anchor) throw new Error('missing download anchor');"
                "anchor.click();"
                "return true;"
                "})()"
            ),
            "target": {"context": context},
            "awaitPromise": False,
        },
    )
    messages = await _recv_until_id(websocket, id_)
    response = messages[-1]
    assert_equal(response["type"], "success", f"{label} type: {response!r}")
    assert_equal(response["result"]["result"]["value"], True, f"{label} result")
    return messages


async def _navigate_complete(
    websocket: Any,
    id_: int,
    context: str,
    url: str,
    label: str,
) -> dict[str, Any]:
    await _send(
        websocket,
        id_,
        "browsingContext.navigate",
        {"context": context, "url": url, "wait": "complete"},
    )
    return await _recv_success(websocket, id_, label)


async def _evaluate_remote_value(
    websocket: Any,
    id_: int,
    context: str,
    expression: str,
    label: str,
) -> dict[str, Any]:
    await _send(
        websocket,
        id_,
        "script.evaluate",
        {
            "expression": expression,
            "target": {"context": context},
            "awaitPromise": False,
        },
    )
    response = await _recv_success(websocket, id_, label)
    return response["result"]["result"]


async def _evaluate_json_object(
    websocket: Any,
    id_: int,
    context: str,
    expression: str,
    label: str,
) -> dict[str, Any]:
    value = await _evaluate_remote_value(websocket, id_, context, expression, label)
    parsed = json.loads(value["value"])
    assert_true(isinstance(parsed, dict), f"{label} should return a JSON object")
    return parsed


async def _get_bidi_cookies(
    websocket: Any,
    id_: int,
    context: str,
    domain: str,
) -> list[dict[str, Any]]:
    await _send(
        websocket,
        id_,
        "storage.getCookies",
        {
            "filter": {
                "name": "bidiSmoke",
                "domain": domain,
                "path": "/",
                "sameSite": "lax",
            },
            "partition": {"type": "context", "context": context},
        },
    )
    response = await _recv_success(websocket, id_, "BiDi storage.getCookies")
    cookies = response["result"]["cookies"]
    assert_true(isinstance(cookies, list), "BiDi storage.getCookies result should include list")
    return cookies


async def _collect_download_events(
    websocket: Any,
    messages: list[dict[str, Any]],
    url: str,
) -> tuple[dict[str, Any], dict[str, Any]]:
    deadline = asyncio.get_running_loop().time() + 10
    will_begin: dict[str, Any] | None = None
    download_end: dict[str, Any] | None = None
    while True:
        for message in messages:
            if message.get("type") != "event":
                continue
            params = message.get("params", {})
            if params.get("url") != url:
                continue
            if message.get("method") == "browsingContext.downloadWillBegin":
                will_begin = message
            elif message.get("method") == "browsingContext.downloadEnd":
                download_end = message
        if will_begin and download_end:
            return will_begin, download_end
        remaining = deadline - asyncio.get_running_loop().time()
        if remaining <= 0:
            raise RuntimeError(f"timed out waiting for BiDi download events for {url}: {messages!r}")
        messages.append(await asyncio.wait_for(_recv(websocket), timeout=remaining))


async def _recv(websocket: Any) -> dict[str, Any]:
    message = await websocket.recv()
    parsed = json.loads(message)
    if not isinstance(parsed, dict):
        raise RuntimeError(f"expected BiDi JSON object, got {parsed!r}")
    return parsed


async def _recv_until_id(websocket: Any, id_: int) -> list[dict[str, Any]]:
    messages: list[dict[str, Any]] = []
    while True:
        message = await _recv(websocket)
        messages.append(message)
        if message.get("id") == id_:
            return messages


async def _find_network_response_completed(
    websocket: Any, messages: list[dict[str, Any]], url: str
) -> dict[str, Any]:
    deadline = asyncio.get_running_loop().time() + 5
    while True:
        for message in messages:
            if (
                message.get("type") == "event"
                and message.get("method") == "network.responseCompleted"
                and message.get("params", {}).get("response", {}).get("url") == url
            ):
                return message
        remaining = deadline - asyncio.get_running_loop().time()
        if remaining <= 0:
            raise RuntimeError(f"timed out waiting for network.responseCompleted for {url}")
        messages.append(await asyncio.wait_for(_recv(websocket), timeout=remaining))


def _find_bidi_event(
    messages: list[dict[str, Any]],
    method: str,
    predicate: Callable[[dict[str, Any]], bool],
) -> dict[str, Any] | None:
    for message in messages:
        if message.get("type") == "event" and message.get("method") == method and predicate(message):
            return message
    return None


async def _recv_until_bidi_event(
    websocket: Any,
    method: str,
    label: str,
    predicate: Callable[[dict[str, Any]], bool],
) -> dict[str, Any]:
    deadline = asyncio.get_running_loop().time() + 5
    seen: list[dict[str, Any]] = []
    while True:
        remaining = deadline - asyncio.get_running_loop().time()
        if remaining <= 0:
            raise RuntimeError(f"timed out waiting for {label}: {seen!r}")
        message = await asyncio.wait_for(_recv(websocket), timeout=remaining)
        seen.append(message)
        if message.get("type") == "event" and message.get("method") == method and predicate(message):
            return message


async def _collect_lifecycle_events(websocket: Any, messages: list[dict[str, Any]], context: str) -> list[dict[str, Any]]:
    # A parent-context subscription includes its descendants in Chrome. The
    # basic fixture has an iframe; check the requested document's lifecycle
    # independently of that child's DOMContentLoaded event.
    lifecycle = [
        message
        for message in messages
        if message.get("type") == "event"
        and message.get("method") == "browsingContext.domContentLoaded"
        and message.get("params", {}).get("context") == context
    ]
    deadline = asyncio.get_running_loop().time() + 5
    while [event["method"] for event in lifecycle] != ["browsingContext.domContentLoaded"]:
        remaining = deadline - asyncio.get_running_loop().time()
        if remaining <= 0:
            return lifecycle
        try:
            message = await asyncio.wait_for(_recv(websocket), timeout=remaining)
        except TimeoutError:
            return lifecycle
        if (
            message.get("type") == "event"
            and message.get("method") == "browsingContext.domContentLoaded"
            and message.get("params", {}).get("context") == context
        ):
            lifecycle.append(message)
    return lifecycle
