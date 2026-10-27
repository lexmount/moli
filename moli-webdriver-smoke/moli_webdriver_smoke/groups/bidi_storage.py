from __future__ import annotations

import asyncio
import json
from functools import partial
from typing import Any
from urllib.parse import urlsplit

import websockets

from ..assertions import assert_equal, assert_true, record
from ..client import ClassicClient, classic_value
from ..config import WebDriverTarget
from ..scenarios import record_failure
from ..selenium_options import create_selenium_options
from .bidi import BidiScenario, _recv_success, _send


async def run_bidi_storage_group(
    target: WebDriverTarget,
    fixture: str,
    results: list[dict[str, Any]],
    continue_on_failure: bool = False,
) -> None:
    scenarios = _bidi_cookie_deletion_scenarios()
    if continue_on_failure:
        for name, scenario in scenarios:
            try:
                await _run_bidi_storage_session(target, fixture, results, ((name, scenario),))
            except Exception as error:
                record_failure(results, "bidi-storage", name, error)
        return
    await _run_bidi_storage_session(target, fixture, results, scenarios)


async def _run_bidi_storage_session(
    target: WebDriverTarget,
    fixture: str,
    results: list[dict[str, Any]],
    scenarios: tuple[tuple[str, BidiScenario], ...],
) -> None:
    # Attach through the standard capability so exactly the same wire scenarios
    # can run against Moli and an external ChromeDriver.
    options = create_selenium_options(target)
    options.set_capability("webSocketUrl", True)
    client = ClassicClient(target.endpoint)
    created = classic_value(client.post("/session", {"capabilities": {"alwaysMatch": options.to_capabilities()}}))
    session_id = created["sessionId"]
    capabilities = created["capabilities"]
    scenario_error: Exception | None = None
    try:
        version = capabilities.get("browserVersion")
        assert_true(isinstance(version, str) and bool(version), "BiDi storage browserVersion")
        async with websockets.connect(capabilities["webSocketUrl"], max_size=2**24) as websocket:
            async with asyncio.timeout(10):
                await _send(websocket, 1, "browsingContext.getTree", {})
                tree = await _recv_success(websocket, 1, "BiDi storage initial context")
            contexts = tree["result"]["contexts"]
            assert_true(bool(contexts), "BiDi storage requires an initial browsing context")
            context = contexts[0]["context"]
            for _name, scenario in scenarios:
                observed: list[dict[str, Any]] = []
                await scenario(websocket, context, fixture, observed)
                for entry in observed:
                    entry.update(browserName=capabilities["browserName"], browserVersion=capabilities["browserVersion"])
                results.extend(observed)
    except Exception as error:
        scenario_error = error
    finally:
        try:
            client.delete(f"/session/{session_id}")
        except Exception as error:
            if scenario_error is None:
                scenario_error = error
    if scenario_error is not None:
        raise scenario_error


def _bidi_cookie_deletion_scenarios() -> tuple[tuple[str, BidiScenario], ...]:
    name = "bidiDeleteSmoke"
    cases = (
        ("unpartitioned_value", {"value": {"type": "string", "value": "plain"}}, ("plain",)),
        ("partition_a_value", {"value": {"type": "string", "value": "part-a"}}, ("part-a",)),
        ("partition_b_value", {"value": {"type": "string", "value": "part-b"}}, ("part-b",)),
        (
            "compound_attributes",
            {"name": name, "domain": "127.0.0.1", "path": "/", "secure": True, "httpOnly": True, "sameSite": "none"},
            ("part-a",),
        ),
        ("size", {"size": len(name) + len("part-a")}, ("part-a", "part-b")),
        ("path", {"path": "/app"}, ("app",)),
        ("no_match_value", {"value": {"type": "string", "value": "missing"}}, ()),
        ("no_match_path", {"path": "/ap"}, ()),
        ("empty_filter", {}, ("plain", "part-a", "part-b", "app", "unrelated")),
        ("omitted_filter", None, ("plain", "part-a", "part-b", "app", "unrelated")),
    )
    return tuple(
        (
            f"bidi_cookie_delete_{label}",
            partial(_run_bidi_cookie_deletion_smoke, label=label, cookie_filter=cookie_filter, deleted_values=values),
        )
        for label, cookie_filter, values in cases
    )


async def _run_bidi_cookie_deletion_smoke(
    websocket: Any,
    default_context: str,
    fixture: str,
    results: list[dict[str, Any]],
    *,
    label: str,
    cookie_filter: dict[str, Any] | None,
    deleted_values: tuple[str, ...],
) -> None:
    name = "bidiDeleteSmoke"
    host = urlsplit(fixture).hostname
    command_id = 200

    async def command(method: str, params: dict[str, Any]) -> dict[str, Any]:
        nonlocal command_id
        command_id += 1
        try:
            async with asyncio.timeout(10):
                await _send(websocket, command_id, method, params)
                response = await _recv_success(websocket, command_id, f"BiDi cookie deletion {label}: {method}")
                return response["result"]
        except TimeoutError as error:
            raise RuntimeError(f"BiDi cookie deletion {label}: timed out on {method} (id={command_id})") from error

    async def set_cookie(context: str, value: str, *, cookie_name: str = name, path: str = "/") -> None:
        await command(
            "storage.setCookie",
            {
                "cookie": {
                    "name": cookie_name,
                    "value": {"type": "string", "value": value},
                    "domain": host,
                    "path": path,
                    "secure": True,
                    "httpOnly": False,
                    "sameSite": "none",
                },
                "partition": {"type": "context", "context": context},
            },
        )

    async def cookies(context: str) -> list[dict[str, Any]]:
        response = await command("storage.getCookies", {"partition": {"type": "context", "context": context}})
        return sorted(response["cookies"], key=lambda cookie: json.dumps(cookie, sort_keys=True))

    user_context = (await command("browser.createUserContext", {}))["userContext"]
    sentinel_set = False
    scenario_error: Exception | None = None
    try:
        context = (await command("browsingContext.create", {"type": "tab", "userContext": user_context}))["context"]
        partition = {"type": "context", "context": context}
        await command(
            "browsingContext.navigate",
            {"context": context, "url": f"{fixture}/webdriver/partitioned-cookie-top", "wait": "complete"},
        )
        await set_cookie(context, "plain")
        await set_cookie(context, "app", path="/app")
        await set_cookie(context, "unrelated", cookie_name="bidiDeleteUnrelated")

        # The same response host writes a second CHIPS cookie under another top-level site.
        # Both hosts are loopback origins, so Secure cookies work without an external TLS service.
        other_site = fixture.replace("127.0.0.1", "localhost", 1)
        await command(
            "browsingContext.navigate",
            {"context": context, "url": f"{other_site}/webdriver/frame", "wait": "complete"},
        )
        frame_url = json.dumps(f"{fixture}/webdriver/partitioned-cookie-frame")
        frame = await command(
            "script.evaluate",
            {
                "expression": (
                    "new Promise((resolve, reject) => {"
                    "const frame = document.createElement('iframe');"
                    "frame.onload = () => resolve('loaded');"
                    "frame.onerror = () => reject(new Error('cookie frame failed'));"
                    f"frame.src = {frame_url};"
                    "document.body.appendChild(frame);"
                    "})"
                ),
                "target": {"context": context},
                "awaitPromise": True,
            },
        )
        assert_equal(frame["type"], "success", f"BiDi cookie fixture script: {frame!r}")
        assert_equal(frame["result"], {"type": "string", "value": "loaded"}, "BiDi cookie frame loaded")

        before = await cookies(context)
        seeded = {cookie["value"]["value"]: cookie for cookie in before}
        assert_equal(len(before), 5, "BiDi cookie deletion fixture count")
        assert_equal(set(seeded), {"plain", "part-a", "part-b", "app", "unrelated"}, "BiDi cookie fixture values")
        for value, cookie in seeded.items():
            assert_equal(cookie["name"], "bidiDeleteUnrelated" if value == "unrelated" else name, "BiDi fixture name")
            assert_equal(cookie["domain"], host, "BiDi fixture domain")
            assert_equal(cookie["path"], "/app" if value == "app" else "/", "BiDi fixture path")
            assert_equal(cookie["httpOnly"], value == "part-a", "BiDi fixture httpOnly")
            assert_equal(cookie["secure"], True, "BiDi fixture secure")
            assert_equal(cookie["sameSite"], "none", "BiDi fixture sameSite")

        await set_cookie(default_context, "sentinel")
        sentinel_set = True
        default_before = await cookies(default_context)
        assert_true(
            any(cookie["name"] == name and cookie["value"]["value"] == "sentinel" for cookie in default_before),
            "BiDi default context sentinel should exist",
        )

        params: dict[str, Any] = {"partition": partition}
        if cookie_filter is not None:
            params["filter"] = cookie_filter
        selection = await command("storage.getCookies", params)
        selected = selection["cookies"]
        assert_equal(
            sorted(cookie["value"]["value"] for cookie in selected),
            sorted(deleted_values),
            f"BiDi {label} selected cookies",
        )
        deleted = await command("storage.deleteCookies", params)
        assert_equal(deleted["partitionKey"], selection["partitionKey"], "BiDi get/delete resolve the same partitionKey")
        expected = [cookie for cookie in before if cookie["value"]["value"] not in deleted_values]
        assert_equal(await cookies(context), expected, f"BiDi {label} preserves all unselected cookie identities")
        assert_equal(await cookies(default_context), default_before, f"BiDi {label} preserves another user context")
    except Exception as error:
        scenario_error = error
    finally:
        cleanup = [("browser.removeUserContext", {"userContext": user_context})]
        if sentinel_set:
            cleanup.append(
                (
                    "storage.deleteCookies",
                    {
                        "filter": {"name": name, "value": {"type": "string", "value": "sentinel"}},
                        "partition": {"type": "context", "context": default_context},
                    },
                )
            )
        for method, params in cleanup:
            try:
                await command(method, params)
            except Exception as error:
                if scenario_error is None:
                    scenario_error = error
    if scenario_error is not None:
        raise scenario_error
    record(results, f"bidi_cookie_delete_{label}", {"deletedValues": list(deleted_values), "seededCookies": 5})
