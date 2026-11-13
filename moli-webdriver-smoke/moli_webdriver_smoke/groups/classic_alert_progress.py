"""Classic alert commands interrupt navigation wait only for an actual prompt."""

from __future__ import annotations

import asyncio
import http.client
import json
import threading
from typing import Any
from urllib.parse import urlsplit

from ..assertions import assert_equal, assert_true, record
from ..client import ClassicClient
from ..config import WebDriverTarget
from ..scenarios import record_failure
from .bidi_network import _bidi_connection, _held_navigation
from .bidi_progress import _Wire


def _alert_request(
    endpoint: str, path: str, operation: str, sent: threading.Event
) -> dict[str, Any]:
    # Keep the response pending without cancelling the HTTP request when the
    # test observes silence. `sent` means the request bytes have been written.
    address = urlsplit(endpoint)
    connection = http.client.HTTPConnection(address.hostname, address.port, timeout=10)
    method = "GET" if operation == "get" else "POST"
    suffix = "text" if operation in ("get", "set") else operation
    body = {"text": "Classic text"} if operation == "set" else {}
    try:
        connection.request(
            method,
            f"{address.path}{path}/alert/{suffix}",
            body=None if method == "GET" else json.dumps(body),
            headers={"Content-Type": "application/json"},
        )
        sent.set()
        response = connection.getresponse()
        return {
            "operation": operation,
            "status": response.status,
            "body": json.loads(response.read()),
        }
    finally:
        connection.close()


async def _scenario(
    target: WebDriverTarget,
    fixture: str,
    results: list[dict[str, Any]],
    name: str,
    timing: str,
    operation: str,
) -> None:
    trace: list[dict[str, Any]] = []
    with (
        _held_navigation() as (trigger, trigger_arrived, trigger_release),
        _held_navigation() as (held, arrived, release),
    ):
        async with _bidi_connection(
            target,
            name,
            initial_url=f"{fixture}/webdriver/frame",
            unhandled_prompt_behavior="ignore",
        ) as (bidi, capabilities):
            session = capabilities["webSocketUrl"].rstrip("/").rsplit("/", 1)[-1]
            path = f"/session/{session}"
            wire = _Wire(bidi.websocket, name, session)
            client = ClassicClient(target.endpoint)
            request: asyncio.Task[dict[str, Any]] | None = None
            try:
                context = (await wire.call("browsingContext.getTree", {}))["contexts"][
                    0
                ]["context"]
                await asyncio.to_thread(
                    client.post, path + "/window", {"handle": context}
                )
                assert_equal(
                    (await asyncio.to_thread(client.get, path + "/window"))["value"],
                    context,
                    f"{name}: Classic and BiDi address the same Page",
                )
                await wire.call(
                    "session.subscribe",
                    {
                        "events": [
                            "browsingContext.userPromptOpened",
                            "browsingContext.userPromptClosed",
                            "log.entryAdded",
                        ],
                        "contexts": [context],
                    },
                )
                if timing != "absent":
                    await wire.evaluate(
                        context,
                        "void(async()=>{await fetch("
                        + json.dumps(trigger)
                        + ",{mode:'no-cors'});const value=prompt('Classic prompt','');"
                        "console.log('Classic result:'+JSON.stringify(value));})()",
                    )
                    assert_true(
                        await asyncio.to_thread(trigger_arrived.wait, 10),
                        f"{name}: old Document armed its prompt",
                    )
                if timing != "normal":
                    assert_equal(
                        await wire.evaluate(
                            context, "location.href=" + json.dumps(held) + ";42"
                        ),
                        {"type": "number", "value": 42},
                        f"{name}: script initiated navigation and already replied",
                    )
                    assert_true(
                        await asyncio.to_thread(arrived.wait, 10),
                        f"{name}: navigation reached the HTTP gate",
                    )
                if timing in ("normal", "existing"):
                    trigger_release.set()
                    await _prompt_opened(wire, context)

                sent = threading.Event()
                request = asyncio.create_task(
                    asyncio.to_thread(
                        _alert_request, target.endpoint, path, operation, sent
                    )
                )
                assert_true(
                    await asyncio.to_thread(sent.wait, 10), f"{name}: request sent"
                )
                if timing in ("arriving", "absent"):
                    # Bounded absence observation, not a sleep used to make an
                    # event happen. Native tests also poll the actual wait before
                    # installing a dialog, without a wall-clock assumption.
                    done, _ = await asyncio.wait({request}, timeout=0.25)
                    assert_true(
                        not done, f"{name}: no dialog still means navigation wait"
                    )
                    assert_true(
                        not release.is_set(), f"{name}: HTTP gate remains closed"
                    )
                    if timing == "arriving":
                        trigger_release.set()
                    else:
                        release.set()

                response = await request
                trace.append(response)
                if timing == "absent":
                    assert_equal(
                        response["status"], 404, f"{name}: missing dialog HTTP status"
                    )
                    assert_equal(
                        response["body"]["value"]["error"], "no such alert", name
                    )
                else:
                    assert_equal(
                        response["status"],
                        200,
                        f"{name}: alert operation: {response!r}",
                    )
                    if operation == "get":
                        assert_equal(response["body"]["value"], "Classic prompt", name)
                        trace.append(
                            await asyncio.to_thread(
                                _alert_request,
                                target.endpoint,
                                path,
                                "set",
                                threading.Event(),
                            )
                        )
                        assert_equal(trace[-1]["status"], 200, f"{name}: fill prompt")
                    if operation in ("get", "set"):
                        trace.append(
                            await asyncio.to_thread(
                                _alert_request,
                                target.endpoint,
                                path,
                                "accept",
                                threading.Event(),
                            )
                        )
                        assert_equal(
                            trace[-1]["status"], 200, f"{name}: accept filled prompt"
                        )
                    expected = (
                        None
                        if operation == "dismiss"
                        else ("Classic text" if operation in ("get", "set") else "")
                    )
                    await wire.until(
                        lambda m: m.get("method") == "log.entryAdded"
                        and m["params"].get("text")
                        == "Classic result:" + json.dumps(expected),
                        "the old script receives the actual prompt return value",
                    )
                    opened = [
                        m
                        for m in wire.messages
                        if m.get("method") == "browsingContext.userPromptOpened"
                        and m["params"].get("context") == context
                    ]
                    assert_equal(len(opened), 1, f"{name}: exactly one prompt opening")
                    assert_true(
                        not release.is_set(),
                        f"{name}: operations finish before navigation",
                    )
                    release.set()
                assert_equal(
                    (await asyncio.to_thread(client.get, path + "/title"))["value"],
                    "Frame" if timing == "normal" else "held-navigation-completed",
                    f"{name}: navigation and Classic session remain usable",
                )
                record(
                    results,
                    name,
                    {
                        "capabilities": capabilities,
                        "http": trace,
                        "requests": wire.requests,
                        "messages": wire.messages,
                    },
                )
            except Exception as error:
                error.add_note(
                    json.dumps(
                        {
                            "http": trace,
                            "requests": wire.requests,
                            "messages": wire.messages,
                        },
                        ensure_ascii=False,
                    )
                )
                raise
            finally:
                # The verdict above is final before cleanup releases either gate.
                trigger_release.set()
                release.set()
                if request is not None:
                    await asyncio.gather(request, return_exceptions=True)


async def _prompt_opened(wire: _Wire, context: str) -> None:
    await wire.until(
        lambda m: m.get("method") == "browsingContext.userPromptOpened"
        and m["params"].get("context") == context
        and m["params"].get("message") == "Classic prompt",
        "old Document's prompt",
    )


async def run_classic_alert_progress_group(
    target: WebDriverTarget,
    fixture: str,
    results: list[dict[str, Any]],
    continue_on_failure: bool = False,
) -> None:
    for timing in ("normal", "existing", "arriving", "absent"):
        for operation in ("get", "set", "accept", "dismiss"):
            name = f"classic_alert_{timing}_{operation}"
            try:
                await _scenario(target, fixture, results, name, timing, operation)
            except Exception as error:
                if not continue_on_failure:
                    raise
                record_failure(results, "classic-alert-progress", name, error)
