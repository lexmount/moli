from __future__ import annotations

import asyncio
import json
import time
from dataclasses import dataclass
from functools import partial
from typing import Any, Awaitable, Callable
from urllib.parse import urlsplit

import websockets

from ..assertions import assert_equal, assert_true, record
from ..client import ClassicClient, classic_value
from ..config import WebDriverTarget
from ..scenarios import record_failure
from ..selenium_options import create_selenium_options
from .bidi import BidiScenario, _recv_success, _recv_until_id, _send


@dataclass
class CookieFixture:
    command: Callable[..., Awaitable[dict[str, Any]]]
    context: str
    user_context: str
    fixture: str
    before: list[dict[str, Any]]

    @property
    def partition(self) -> dict[str, Any]:
        return {"type": "context", "context": self.context}

    def storage_key(self, **attributes: Any) -> dict[str, Any]:
        return {"type": "storageKey", "userContext": self.user_context, **attributes}

    async def cookies(self) -> list[dict[str, Any]]:
        response = await self.command("storage.getCookies", {"partition": self.partition})
        return sorted(response["cookies"], key=lambda cookie: json.dumps(cookie, sort_keys=True))


CookieCheck = Callable[[CookieFixture], Awaitable[dict[str, Any]]]


async def run_bidi_storage_group(
    target: WebDriverTarget,
    fixture: str,
    results: list[dict[str, Any]],
    continue_on_failure: bool = False,
) -> None:
    scenarios = _bidi_storage_scenarios()
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


def _bidi_storage_scenarios() -> tuple[tuple[str, BidiScenario], ...]:
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
    scenarios = tuple(
        (
            f"bidi_cookie_delete_{label}",
            partial(_run_bidi_cookie_scenario, label=label, cookie_filter=cookie_filter, deleted_values=values),
        )
        for label, cookie_filter, values in cases
    )
    checks = (
        (_check_storage_key_filter, "storage_key", (
            "all", "site_a", "site_b", "nonexistent", "other_scheme",
            "ancestor_false", "ancestor_true", "wrong_ancestor",
        )),
        (_check_storage_key_invalid, "storage_key_invalid", (
            "url", "null", "number", "ancestor", "user_context", "context",
        )),
        (_check_storage_key_set, "storage_set", (
            "site", "file", "ancestor_without_origin", "opaque",
        )),
        (_check_cookie_attribute, "delete", ("expiry", "unicode_value")),
    )
    return scenarios + tuple(
        (
            f"bidi_cookie_{prefix}_{kind}",
            partial(
                _run_bidi_cookie_scenario,
                label=f"{prefix}_{kind}",
                check=partial(check, kind=kind),
            ),
        )
        for check, prefix, kinds in checks
        for kind in kinds
    )


async def _run_bidi_cookie_scenario(
    websocket: Any,
    default_context: str,
    fixture: str,
    results: list[dict[str, Any]],
    *,
    label: str,
    cookie_filter: dict[str, Any] | None = None,
    deleted_values: tuple[str, ...] = (),
    check: CookieCheck | None = None,
) -> None:
    name = "bidiDeleteSmoke"
    host = urlsplit(fixture).hostname
    command_id = 200

    async def command(method: str, params: dict[str, Any], expected_error: str | None = None) -> dict[str, Any]:
        nonlocal command_id
        command_id += 1
        try:
            async with asyncio.timeout(10):
                await _send(websocket, command_id, method, params)
                if expected_error is not None:
                    response = (await _recv_until_id(websocket, command_id))[-1]
                    assert_equal(response["type"], "error", f"BiDi {label} {method}: {response!r}")
                    assert_equal(response["error"], expected_error, f"BiDi {label} {method} error")
                    assert_true(isinstance(response.get("message"), str) and bool(response["message"]), "BiDi error message")
                    return response
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

        observed: dict[str, Any] = {}
        if check is not None:
            observed = await check(CookieFixture(command, context, user_context, fixture, before))
        else:
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
    record_name = f"bidi_cookie_{label}" if check is not None else f"bidi_cookie_delete_{label}"
    record(results, record_name, {"deletedValues": list(deleted_values), "seededCookies": 5, **observed})


async def _check_storage_key_filter(env: CookieFixture, *, kind: str) -> dict[str, Any]:
    site_a = f"{urlsplit(env.fixture).scheme}://{urlsplit(env.fixture).hostname}"
    site_b = site_a.replace("127.0.0.1", "localhost")
    cases = {
        "all": ({}, ("plain", "part-a", "part-b", "app", "unrelated"), {}),
        "site_a": ({"sourceOrigin": env.fixture + "/ignored?query=yes"}, ("part-a",), {"sourceOrigin": site_a}),
        "site_b": ({"sourceOrigin": env.fixture.replace("127.0.0.1", "localhost")}, ("part-b",), {"sourceOrigin": site_b}),
        "nonexistent": ({"sourceOrigin": "https://missing.invalid"}, (), {"sourceOrigin": "https://missing.invalid"}),
        "other_scheme": ({"sourceOrigin": "https://127.0.0.1"}, (), {"sourceOrigin": "https://127.0.0.1"}),
        "ancestor_false": ({"goog:hasCrossSiteAncestor": False}, ("part-a",), {"goog:hasCrossSiteAncestor": False}),
        "ancestor_true": ({"goog:hasCrossSiteAncestor": True}, ("part-b",), {"goog:hasCrossSiteAncestor": True}),
        "wrong_ancestor": (
            {"sourceOrigin": env.fixture, "goog:hasCrossSiteAncestor": True}, (),
            {"sourceOrigin": site_a, "goog:hasCrossSiteAncestor": True},
        ),
    }
    attributes, values, expanded = cases[kind]
    params = {"partition": env.storage_key(**attributes)}
    selected = await env.command("storage.getCookies", params)
    assert_equal(sorted(cookie["value"]["value"] for cookie in selected["cookies"]), sorted(values), f"BiDi {kind} selection")
    expected_key = {"userContext": env.user_context, **expanded}
    assert_equal(selected["partitionKey"], expected_key, f"BiDi {kind} expanded storage key")
    deleted = await env.command("storage.deleteCookies", params)
    assert_equal(deleted["partitionKey"], expected_key, f"BiDi {kind} delete storage key")
    expected = [cookie for cookie in env.before if cookie["value"]["value"] not in values]
    assert_equal(await env.cookies(), expected, f"BiDi {kind} preserves unselected storage partitions")
    return {"selectedValues": list(values)}


async def _check_storage_key_invalid(env: CookieFixture, *, kind: str) -> dict[str, Any]:
    descriptor = env.storage_key()
    expected_error = "invalid argument"
    if kind == "context":
        descriptor = {"type": "context", "context": "missing-cookie-context"}
        expected_error = "no such frame"
    elif kind == "ancestor":
        descriptor["goog:hasCrossSiteAncestor"] = "false"
    elif kind == "user_context":
        descriptor["userContext"] = "missing-cookie-user-context"
        expected_error = "no such user context"
    else:
        descriptor["sourceOrigin"] = {"url": "not a URL", "null": None, "number": 42}[kind]
    for method in ("storage.getCookies", "storage.setCookie", "storage.deleteCookies"):
        params: dict[str, Any] = {"partition": descriptor}
        if method == "storage.setCookie":
            params["cookie"] = {
                "name": "bidiDeleteSmoke", "value": {"type": "string", "value": "must-not-write"},
                "domain": "127.0.0.1", "path": "/", "secure": True, "sameSite": "none",
            }
        await env.command(method, params, expected_error=expected_error)
        assert_equal(await env.cookies(), env.before, f"BiDi {method} {kind} leaves stored cookies unchanged")
    return {"error": expected_error, "commandsChecked": 3}


async def _check_storage_key_set(env: CookieFixture, *, kind: str) -> dict[str, Any]:
    cookie = {
        "name": "bidiPartitionSet", "value": {"type": "string", "value": "global"},
        "domain": "127.0.0.1", "path": "/", "secure": True, "sameSite": "none",
    }
    await env.command("storage.setCookie", {"partition": env.partition, "cookie": cookie})
    before = await env.cookies()
    assert_equal(len(before), 6, "BiDi set fixture includes unpartitioned identity")
    cookie["value"] = {"type": "string", "value": "created"}
    attributes: dict[str, Any] = {"goog:hasCrossSiteAncestor": kind == "site"}
    if kind == "site":
        attributes["sourceOrigin"] = env.fixture
        normalized = "http://127.0.0.1"
    elif kind == "file":
        attributes["sourceOrigin"] = "file:///tmp/bidi-cookie-origin"
        normalized = "file://"
    elif kind == "opaque":
        attributes["sourceOrigin"] = "data:text/plain,cookie-origin"
    params = {"partition": env.storage_key(**attributes), "cookie": cookie}
    if kind in ("ancestor_without_origin", "opaque"):
        await env.command("storage.setCookie", params, expected_error="unable to set cookie")
        assert_equal(await env.cookies(), before, "BiDi failed partitioned set preserves every cookie")
        return {"error": "unable to set cookie"}
    result = await env.command("storage.setCookie", params)
    expected_key = {"userContext": env.user_context, "sourceOrigin": normalized, "goog:hasCrossSiteAncestor": attributes["goog:hasCrossSiteAncestor"]}
    assert_equal(result["partitionKey"], expected_key, "BiDi set expanded storage key")
    selected = await env.command("storage.getCookies", {"partition": params["partition"], "filter": {"name": cookie["name"]}})
    assert_equal([value["value"]["value"] for value in selected["cookies"]], ["created"], "BiDi set writes requested partition")
    assert_equal(selected["partitionKey"], expected_key, "BiDi set/get storage key")
    after = await env.cookies()
    assert_equal([value for value in after if value["value"]["value"] != "created"], before, "BiDi partitioned set preserves unpartitioned identity")
    await env.command("storage.deleteCookies", {"partition": params["partition"], "filter": {"name": cookie["name"]}})
    assert_equal(await env.cookies(), before, "BiDi exact partitioned set/delete preserves existing cookies")
    return {"sourceOrigin": normalized}


async def _check_cookie_attribute(env: CookieFixture, *, kind: str) -> dict[str, Any]:
    expiry = int(time.time()) + 3600
    values = ("keep", "café", "你好")
    for index, (partition, value) in enumerate(zip(
        (env.partition, env.storage_key(sourceOrigin=env.fixture), env.storage_key(sourceOrigin=env.fixture.replace("127.0.0.1", "localhost"))),
        values,
    )):
        await env.command("storage.setCookie", {
            "partition": partition,
            "cookie": {
                "name": "bidiAttribute", "value": {"type": "string", "value": value},
                "domain": "127.0.0.1", "path": "/", "secure": True, "sameSite": "none", "expiry": expiry + index,
            },
        })
    before = await env.cookies()
    assert_equal(len(before), 8, "BiDi attribute fixture preserves three partition identities")
    for index, value in enumerate(values):
        cookie = next(cookie for cookie in before if cookie["name"] == "bidiAttribute" and cookie["value"]["value"] == value)
        assert_equal(cookie["expiry"], expiry + index, "BiDi cookie expiry round trip")
        assert_equal(cookie["size"], len("bidiAttribute".encode()) + len(value.encode()), "BiDi cookie size counts UTF-8 bytes")
    cookie_filter: dict[str, Any] = {"name": "bidiAttribute"}
    target = "café" if kind == "expiry" else "你好"
    cookie_filter.update({"expiry": expiry + 1} if kind == "expiry" else {"value": {"type": "string", "value": target}})
    params = {"partition": env.storage_key(), "filter": cookie_filter}
    selected = await env.command("storage.getCookies", params)
    assert_equal([cookie["value"]["value"] for cookie in selected["cookies"]], [target], f"BiDi {kind} selection")
    await env.command("storage.deleteCookies", params)
    expected = [cookie for cookie in before if cookie["value"]["value"] != target]
    assert_equal(await env.cookies(), expected, f"BiDi {kind} preserves all unselected identities")
    return {"selectedValue": target}
