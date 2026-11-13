"""Command progress contracts: real gates and pipelined commands, no retries."""

from __future__ import annotations

import asyncio
import json
from contextlib import asynccontextmanager
from typing import Any, AsyncIterator, Callable

from ..assertions import assert_equal, assert_true, record
from ..client import ClassicClient, WebDriverHttpError
from ..config import WebDriverTarget
from ..scenarios import record_failure
from .bidi_network import _bidi_connection, _held_navigation


class _Wire:
    def __init__(self, socket: Any, name: str, session_id: str) -> None:
        self.socket, self.name = socket, name
        self.session_id = session_id
        self.requests: dict[int, dict[str, Any]] = {}
        self.messages: list[dict[str, Any]] = []

    async def send(self, method: str, params: dict[str, Any]) -> int:
        rid = len(self.requests) + 1
        message = {"id": rid, "method": method, "params": params}
        self.requests[rid] = message
        await self.socket.send(json.dumps(message))
        return rid

    def has_response(self, rid: int) -> bool:
        return any(m.get("id") == rid for m in self.messages)

    async def until(
        self, predicate: Callable[[dict[str, Any]], bool], label: str, *, after: int = 0
    ) -> dict[str, Any]:
        try:
            async with asyncio.timeout(10):
                for message in self.messages[after:]:
                    if predicate(message):
                        return message
                while True:
                    message = json.loads(await self.socket.recv())
                    rid = message.get("id")
                    if isinstance(rid, int):
                        assert_true(
                            rid in self.requests,
                            f"{self.name}: unknown reply: {message!r}",
                        )
                        assert_true(
                            not self.has_response(rid),
                            f"{self.name}: repeated terminal: {message!r}",
                        )
                    self.messages.append(message)
                    if predicate(message):
                        return message
        except TimeoutError as error:
            raise RuntimeError(
                f"{self.name}: waiting for {label}; messages={self.messages[-20:]!r}"
            ) from error

    async def response(self, rid: int, *, allow_error: bool = False) -> dict[str, Any]:
        response = await self.until(
            lambda m: m.get("id") == rid, self.requests[rid]["method"]
        )
        if not allow_error:
            assert_equal(response["type"], "success", f"{self.name}: {response!r}")
        return response

    async def call(self, method: str, params: dict[str, Any]) -> dict[str, Any]:
        return (await self.response(await self.send(method, params)))["result"]

    async def evaluate(self, context: str, expression: str) -> dict[str, Any]:
        result = await self.call(
            "script.evaluate",
            {
                "target": {"context": context},
                "expression": expression,
                "awaitPromise": True,
            },
        )
        assert_equal(
            result["type"], "success", f"{self.name}: script result: {result!r}"
        )
        return result["result"]


@asynccontextmanager
async def _connection(
    target: WebDriverTarget,
    fixture: str,
    results: list[dict[str, Any]],
    name: str,
    *,
    prompt_policy: str | None = None,
    isolated: bool = False,
) -> AsyncIterator[tuple[_Wire, str, str]]:
    async with _bidi_connection(
        target,
        name,
        initial_url=f"{fixture}/webdriver/frame",
        unhandled_prompt_behavior=prompt_policy,
    ) as (bidi, capabilities):
        wire = _Wire(
            bidi.websocket,
            name,
            capabilities["webSocketUrl"].rstrip("/").rsplit("/", 1)[-1],
        )
        a = (await wire.call("browsingContext.getTree", {}))["contexts"][0]["context"]
        create: dict[str, Any] = {"type": "tab", "background": True}
        if isolated:
            create["userContext"] = (await wire.call("browser.createUserContext", {}))[
                "userContext"
            ]
        b = (await wire.call("browsingContext.create", create))["context"]
        await wire.call(
            "browsingContext.navigate",
            {"context": b, "url": f"{fixture}/webdriver/frame", "wait": "complete"},
        )
        try:
            yield wire, a, b
        except Exception as error:
            error.add_note(
                json.dumps(
                    {"requests": wire.requests, "messages": wire.messages},
                    ensure_ascii=False,
                )
            )
            raise
        else:
            record(
                results,
                name,
                {
                    "capabilities": capabilities,
                    "requests": wire.requests,
                    "messages": wire.messages,
                },
            )


async def _navigation(
    target: WebDriverTarget,
    fixture: str,
    results: list[dict[str, Any]],
    name: str,
    wait: str,
) -> None:
    with _held_navigation() as (held, arrived, release):
        async with _connection(target, fixture, results, name) as (wire, a, b):
            try:
                navigation = await wire.send(
                    "browsingContext.navigate",
                    {"context": a, "url": held, "wait": wait},
                )
                assert_true(
                    await asyncio.to_thread(arrived.wait, 10),
                    f"{name}: navigation request arrived",
                )
                close = await wire.send("browsingContext.close", {"context": a})
                tree = await wire.send("browsingContext.getTree", {})
                await wire.response(close)
                await wire.response(tree)
                await wire.response(navigation, allow_error=True)
                assert_true(
                    not release.is_set(),
                    f"{name}: controls complete with the HTTP gate closed",
                )
                assert_equal(
                    await wire.evaluate(b, "6 * 7"),
                    {"type": "number", "value": 42},
                    f"{name}: independent Page",
                )
            finally:
                release.set()


async def _runtime(
    target: WebDriverTarget, fixture: str, results: list[dict[str, Any]], name: str
) -> None:
    async with _connection(target, fixture, results, name) as (wire, a, b):
        await wire.call(
            "session.subscribe", {"events": ["log.entryAdded"], "contexts": [a]}
        )
        pending = await wire.send(
            "script.evaluate",
            {
                "target": {"context": a},
                "awaitPromise": True,
                "expression": "new Promise(resolve => { globalThis.progressResolve = resolve; console.log('progress-promise-started'); })",
            },
        )
        await wire.until(
            lambda m: m.get("method") == "log.entryAdded"
            and m["params"].get("text") == "progress-promise-started",
            "pending promise started",
        )
        assert_equal(
            await wire.evaluate(b, "6 * 7"),
            {"type": "number", "value": 42},
            f"{name}: B executes while A awaits",
        )
        assert_true(
            not wire.has_response(pending),
            f"{name}: unresolved Promise has no terminal",
        )
        assert_equal(
            await wire.evaluate(a, "progressResolve(17); true"),
            {"type": "boolean", "value": True},
            f"{name}: same-context resolver",
        )
        result = (await wire.response(pending))["result"]
        assert_equal(
            result["result"],
            {"type": "number", "value": 17},
            f"{name}: original Promise result",
        )


async def _dialog_during_pending_navigation(
    target: WebDriverTarget, fixture: str, results: list[dict[str, Any]], name: str
) -> None:
    # Open the old Document's prompt only after the navigation starts. Chrome
    # can dismiss a prompt that was already open when navigation was requested.
    with (
        _held_navigation() as (trigger, trigger_arrived, trigger_release),
        _held_navigation() as (held, arrived, release),
    ):
        async with _connection(
            target, fixture, results, name, prompt_policy="ignore"
        ) as (wire, a, _b):
            try:
                await wire.call(
                    "session.subscribe",
                    {
                        "events": [
                            "browsingContext.userPromptOpened",
                            "browsingContext.userPromptClosed",
                            "log.entryAdded",
                        ],
                        "contexts": [a],
                    },
                )
                await wire.evaluate(
                    a,
                    "void(async()=>{await fetch("
                    + json.dumps(trigger)
                    + ", {mode:'no-cors'});"
                    "const result=confirm('pending-navigation-prompt');"
                    "console.log('pending-navigation-result:'+result);})()",
                )
                assert_true(
                    await asyncio.to_thread(trigger_arrived.wait, 10),
                    f"{name}: old Document has armed its prompt trigger",
                )
                navigation = await wire.send(
                    "browsingContext.navigate",
                    {"context": a, "url": held, "wait": "complete"},
                )
                assert_true(
                    await asyncio.to_thread(arrived.wait, 10),
                    f"{name}: new navigation reaches the held HTTP response",
                )
                trigger_release.set()
                await wire.until(
                    lambda m: m.get("method") == "browsingContext.userPromptOpened"
                    and m["params"].get("context") == a
                    and m["params"].get("message") == "pending-navigation-prompt",
                    "old Document's prompt during navigation",
                )
                await wire.call(
                    "browsingContext.handleUserPrompt", {"context": a, "accept": True}
                )
                await wire.until(
                    lambda m: m.get("method") == "browsingContext.userPromptClosed"
                    and m["params"].get("context") == a
                    and m["params"].get("accepted") is True,
                    "the original prompt is accepted",
                )
                await wire.until(
                    lambda m: m.get("method") == "log.entryAdded"
                    and m["params"].get("text") == "pending-navigation-result:true",
                    "the old script receives its actual confirm result",
                )
                assert_true(
                    not release.is_set() and not wire.has_response(navigation),
                    f"{name}: prompt handling does not require or settle navigation",
                )
                release.set()
                await wire.response(navigation)
                assert_equal(
                    await wire.evaluate(a, "document.title"),
                    {"type": "string", "value": "held-navigation-completed"},
                    f"{name}: navigation commits after handling the old prompt",
                )
            finally:
                trigger_release.set()
                release.set()


async def _runtime_popup_facts(
    target: WebDriverTarget, fixture: str, results: list[dict[str, Any]], name: str
) -> None:
    async with _connection(target, fixture, results, name) as (wire, a, b):
        await wire.call(
            "session.subscribe",
            {
                "events": [
                    "browsingContext.contextCreated",
                    "browsingContext.load",
                    "log.entryAdded",
                ]
            },
        )
        after = len(wire.messages)
        commands = []
        children = []
        for context, marker in ((a, "a"), (b, "b")):
            url = fixture + "/webdriver/frame?pending-popup=" + marker
            commands.append(
                await wire.send(
                    "script.evaluate",
                    {
                        "target": {"context": context},
                        "awaitPromise": True,
                        "expression": f"window.open({json.dumps(url)}); new Promise(resolve=>{{globalThis.progressResolve=resolve; console.log('popup-pending-{marker}');}})",
                    },
                )
            )
            await wire.until(
                lambda m: m.get("method") == "log.entryAdded"
                and m["params"].get("text") == "popup-pending-" + marker,
                "original evaluation owns an unresolved Promise",
                after=after,
            )
            child = (
                await wire.until(
                    lambda m: m.get("method") == "browsingContext.contextCreated"
                    and m["params"].get("originalOpener") == context,
                    "the popup's Native creation fact",
                    after=after,
                )
            )["params"]["context"]
            children.append(child)
            await wire.until(
                lambda m: m.get("method") == "browsingContext.load"
                and m["params"].get("context") == child
                and m["params"].get("url") == url,
                "actual popup load before the opener RPC resolves",
                after=after,
            )
            assert_true(
                not wire.has_response(commands[-1]),
                f"{name}: popup facts do not settle the original Promise",
            )
        for index in (1, 0):
            await wire.evaluate((a, b)[index], f"progressResolve({index + 31}); true")
            assert_equal(
                (await wire.response(commands[index]))["result"]["result"],
                {"type": "number", "value": index + 31},
                f"{name}: original popup command receipt",
            )
        for child in children:
            assert_equal(
                sum(
                    m.get("method") == "browsingContext.contextCreated"
                    and m["params"].get("context") == child
                    for m in wire.messages[after:]
                ),
                1,
                f"{name}: one creation fact per popup",
            )
            await wire.call("browsingContext.close", {"context": child})


async def _input_pause(
    target: WebDriverTarget, fixture: str, results: list[dict[str, Any]], name: str
) -> None:
    with _held_navigation() as (started, arrived, release):
        async with _connection(target, fixture, results, name) as (wire, a, b):
            try:
                await wire.evaluate(
                    a,
                    "document.body.innerHTML='<input id=progress>'; document.querySelector('input').focus();"
                    f"addEventListener('keydown', () => fetch({json.dumps(started)}), {{once:true}}); true",
                )
                action = await wire.send(
                    "input.performActions",
                    {
                        "context": a,
                        "actions": [
                            {
                                "type": "key",
                                "id": "progress-key",
                                "actions": [
                                    {"type": "keyDown", "value": "x"},
                                    {"type": "pause", "duration": 5000},
                                    {"type": "keyUp", "value": "x"},
                                ],
                            }
                        ],
                    },
                )
                assert_true(
                    await asyncio.to_thread(arrived.wait, 10),
                    f"{name}: first input tick executed",
                )
                await wire.call("browsingContext.getTree", {})
                assert_true(
                    not wire.has_response(action),
                    f"{name}: getTree must not wait for the input pause",
                )
                assert_equal(
                    await wire.evaluate(b, "6 * 7"),
                    {"type": "number", "value": 42},
                    f"{name}: independent script during input pause",
                )
                await wire.call("browsingContext.close", {"context": a})
                await wire.response(action, allow_error=True)
            finally:
                release.set()


async def _auth_navigations(
    target: WebDriverTarget, fixture: str, results: list[dict[str, Any]], name: str
) -> None:
    async with _connection(target, fixture, results, name, isolated=True) as (
        wire,
        a,
        b,
    ):
        await wire.call(
            "session.subscribe",
            {"events": ["network.authRequired"], "contexts": [a, b]},
        )
        pending = []
        paused = []
        for context in (a, b):
            await wire.call(
                "network.addIntercept",
                {"phases": ["authRequired"], "contexts": [context]},
            )
            pending.append(
                await wire.send(
                    "browsingContext.navigate",
                    {
                        "context": context,
                        "url": fixture + "/webdriver/basic-auth?context=" + context,
                        "wait": "complete",
                    },
                )
            )
            paused.append(
                await wire.until(
                    lambda m: m.get("method") == "network.authRequired"
                    and m["params"]["context"] == context,
                    "context-specific authRequired",
                )
            )
        for index in (1, 0):
            await wire.call(
                "network.continueWithAuth",
                {
                    "request": paused[index]["params"]["request"]["request"],
                    "action": "provideCredentials",
                    "credentials": {
                        "type": "password",
                        "username": "postman",
                        "password": "password",
                    },
                },
            )
            await wire.response(pending[index])
        for context in (a, b):
            assert_equal(
                await wire.evaluate(context, "document.title"),
                {"type": "string", "value": "Basic Auth"},
                f"{name}: correct authenticated Document",
            )


async def _input_queue(
    target: WebDriverTarget, fixture: str, results: list[dict[str, Any]], name: str
) -> None:
    async with _connection(target, fixture, results, name) as (wire, a, b):
        await wire.call(
            "session.subscribe", {"events": ["log.entryAdded"], "contexts": [a]}
        )
        await wire.evaluate(
            a,
            "document.body.innerHTML='<input>'; document.querySelector('input').focus();"
            "globalThis.progressKeys=[]; for(const type of ['keydown','keyup']) addEventListener(type,e=>{progressKeys.push(type+':'+e.key+':'+e.shiftKey); if(e.key==='Shift'&&type==='keydown')console.log('input-queue-start');}); true",
        )
        first = await wire.send(
            "input.performActions",
            {
                "context": a,
                "actions": [
                    {
                        "type": "key",
                        "id": "queue-key",
                        "actions": [
                            {"type": "keyDown", "value": "\ue008"},
                            {"type": "pause", "duration": 3000},
                            {"type": "keyDown", "value": "\ue012"},
                        ],
                    }
                ],
            },
        )
        await wire.until(
            lambda m: m.get("method") == "log.entryAdded"
            and m["params"].get("text") == "input-queue-start",
            "first action tick",
        )
        later = await wire.send(
            "input.performActions",
            {
                "context": a,
                "actions": [
                    {
                        "type": "key",
                        "id": "queue-key",
                        "actions": [
                            {"type": "keyUp", "value": "\ue012"},
                            {"type": "keyUp", "value": "\ue008"},
                            {"type": "keyDown", "value": "b"},
                            {"type": "keyUp", "value": "b"},
                        ],
                    }
                ],
            },
        )
        await wire.call(
            "input.performActions",
            {
                "context": b,
                "actions": [
                    {
                        "type": "key",
                        "id": "other-key",
                        "actions": [
                            {"type": "keyDown", "value": "y"},
                            {"type": "keyUp", "value": "y"},
                        ],
                    }
                ],
            },
        )
        assert_true(
            not wire.has_response(first),
            f"{name}: independent input advances during A's pause",
        )
        assert_true(
            not wire.has_response(later), f"{name}: later A action stays queued"
        )
        for command in (first, later):
            await wire.response(command)
        await wire.call("input.releaseActions", {"context": a})
        keys = json.loads(
            (await wire.evaluate(a, "JSON.stringify(progressKeys)"))["value"]
        )
        assert_equal(
            keys,
            [
                "keydown:Shift:true",
                "keydown:ArrowLeft:true",
                "keyup:ArrowLeft:true",
                "keyup:Shift:false",
                "keydown:b:false",
                "keyup:b:false",
            ],
            f"{name}: same-device state and ordered release",
        )


async def _input_release_during_pause(
    target: WebDriverTarget, fixture: str, results: list[dict[str, Any]], name: str
) -> None:
    async with _connection(target, fixture, results, name) as (wire, a, _b):
        await wire.call(
            "session.subscribe", {"events": ["log.entryAdded"], "contexts": [a]}
        )
        await wire.evaluate(
            a,
            "document.body.innerHTML='<input>'; document.querySelector('input').focus();"
            "globalThis.progressKeys=[]; for(const type of ['keydown','keyup']) addEventListener(type,e=>{progressKeys.push(type+':'+e.key);console.log('release-'+type+':'+e.key);}); true",
        )
        action = await wire.send(
            "input.performActions",
            {
                "context": a,
                "actions": [
                    {
                        "type": "key",
                        "id": "release-key",
                        "actions": [
                            {"type": "keyDown", "value": "\ue008"},
                            {"type": "pause", "duration": 5000},
                            {"type": "keyDown", "value": "a"},
                        ],
                    }
                ],
            },
        )
        await wire.until(
            lambda m: m.get("method") == "log.entryAdded"
            and m["params"].get("text") == "release-keydown:Shift",
            "Shift is actually pressed",
        )
        await wire.call("input.releaseActions", {"context": a})
        assert_true(
            not wire.has_response(action),
            f"{name}: release does not wait for the long pause",
        )
        keys = json.loads(
            (await wire.evaluate(a, "JSON.stringify(progressKeys)"))["value"]
        )
        assert_equal(
            keys,
            ["keydown:Shift", "keyup:Shift"],
            f"{name}: no future key state is released",
        )
        await wire.call("browsingContext.close", {"context": a})
        await wire.response(action, allow_error=True)


async def _input_release_generations(
    target: WebDriverTarget, fixture: str, results: list[dict[str, Any]], name: str
) -> None:
    # Calibrated against ChromeDriver 156.0.8078.73. Newer upstream mapper
    # revisions serialize releaseActions on the same actions queue instead.
    async with _connection(target, fixture, results, name) as (wire, a, _b):
        await wire.call(
            "session.subscribe", {"events": ["log.entryAdded"], "contexts": [a]}
        )
        await wire.evaluate(
            a,
            "document.body.innerHTML='<input>';document.querySelector('input').focus();"
            "globalThis.progressKeys=[];for(const type of ['keydown','keyup'])"
            "addEventListener(type,e=>{progressKeys.push([type,e.key,e.shiftKey]);"
            "console.log('generation-'+type+':'+e.key);});true",
        )
        old = await wire.send(
            "input.performActions",
            {
                "context": a,
                "actions": [
                    {
                        "type": "key",
                        "id": "generation-key",
                        "actions": [
                            {"type": "keyDown", "value": "\ue008"},
                            {"type": "pause", "duration": 5000},
                            {"type": "keyDown", "value": "\ue012"},
                            {"type": "keyUp", "value": "\ue012"},
                        ],
                    }
                ],
            },
        )
        await wire.until(
            lambda m: m.get("method") == "log.entryAdded"
            and m["params"].get("text") == "generation-keydown:Shift",
            "the old input generation actually presses Shift",
        )
        await wire.call("input.releaseActions", {"context": a})
        assert_true(
            not wire.has_response(old),
            f"{name}: release completes during the old pause",
        )
        await wire.call(
            "input.performActions",
            {
                "context": a,
                "actions": [
                    {
                        "type": "key",
                        "id": "generation-key",
                        "actions": [
                            {"type": "keyDown", "value": "b"},
                            {"type": "keyUp", "value": "b"},
                        ],
                    }
                ],
            },
        )
        assert_true(
            not wire.has_response(old), f"{name}: new input generation completes first"
        )
        await wire.response(old)
        assert_equal(
            json.loads(
                (await wire.evaluate(a, "JSON.stringify(progressKeys)"))["value"]
            ),
            [
                ["keydown", "Shift", True],
                ["keyup", "Shift", False],
                ["keydown", "b", False],
                ["keyup", "b", False],
                ["keydown", "ArrowLeft", False],
                ["keyup", "ArrowLeft", False],
            ],
            f"{name}: release, new generation, and old continuation retain their input state",
        )


async def _runtime_cancel(
    target: WebDriverTarget, fixture: str, results: list[dict[str, Any]], name: str
) -> None:
    async with _connection(target, fixture, results, name) as (wire, a, b):
        await wire.call("session.subscribe", {"events": ["log.entryAdded"]})
        pending = []
        for context, marker in ((a, "a"), (b, "b")):
            pending.append(
                await wire.send(
                    "script.evaluate",
                    {
                        "target": {"context": context},
                        "awaitPromise": True,
                        "expression": f"new Promise(resolve => {{ globalThis.progressResolve=resolve; console.log('pending-{marker}'); }})",
                    },
                )
            )
            await wire.until(
                lambda m: m.get("method") == "log.entryAdded"
                and m["params"].get("text") == f"pending-{marker}",
                f"{marker} Promise started",
            )
        await wire.call("browsingContext.close", {"context": a})
        await wire.response(pending[0], allow_error=True)
        assert_true(
            not wire.has_response(pending[1]), f"{name}: closing A does not settle B"
        )
        await wire.evaluate(b, "progressResolve(29); true")
        assert_equal(
            (await wire.response(pending[1]))["result"]["result"],
            {"type": "number", "value": 29},
            f"{name}: B owns its result",
        )


async def _input_child_pointer(
    target: WebDriverTarget,
    fixture: str,
    results: list[dict[str, Any]],
    name: str,
    element_origin: bool = False,
) -> None:
    async with _connection(target, fixture, results, name) as (wire, a, _b):
        await wire.evaluate(
            a,
            "new Promise(resolve => { document.body.style.margin='0'; const f=document.createElement('iframe');"
            "f.style='position:absolute;left:100px;top:80px;width:300px;height:180px;border:0'; f.onload=()=>resolve(true);"
            f"f.src={json.dumps(fixture + '/webdriver/frame')}; document.body.append(f); }})",
        )
        child = (await wire.call("browsingContext.getTree", {"root": a}))["contexts"][
            0
        ]["children"][0]["context"]
        await wire.evaluate(
            child,
            "document.body.style.margin='0'; document.body.innerHTML='<button style=\"width:80px;height:60px\">go</button>';"
            "globalThis.progressClicks=0;document.querySelector('button').onclick=()=>progressClicks++; true",
        )
        origin: Any = "viewport"
        if element_origin:
            node = await wire.evaluate(child, "document.querySelector('button')")
            origin = {"type": "element", "element": {"sharedId": node["sharedId"]}}
        await wire.call(
            "input.performActions",
            {
                "context": child,
                "actions": [
                    {
                        "type": "pointer",
                        "id": "frame-pointer",
                        "parameters": {"pointerType": "mouse"},
                        "actions": [
                            {
                                "type": "pointerMove",
                                "origin": origin,
                                "x": 0 if element_origin else 20,
                                "y": 0 if element_origin else 20,
                            },
                            {"type": "pointerDown", "button": 0},
                            {"type": "pointerUp", "button": 0},
                        ],
                    }
                ],
            },
        )
        assert_equal(
            await wire.evaluate(child, "progressClicks"),
            {"type": "number", "value": 1},
            f"{name}: child viewport coordinates reach the child",
        )
        if element_origin:
            invalid = await wire.response(
                await wire.send(
                    "input.performActions",
                    {
                        "context": child,
                        "actions": [
                            {
                                "type": "pointer",
                                "id": "invalid-origin",
                                "actions": [
                                    {
                                        "type": "pointerMove",
                                        "origin": {
                                            "type": "invalid",
                                            "element": {"sharedId": node["sharedId"]},
                                        },
                                        "x": 0,
                                        "y": 0,
                                    },
                                ],
                            }
                        ],
                    },
                ),
                allow_error=True,
            )
            assert_equal(
                invalid["error"],
                "invalid argument",
                f"{name}: malformed element origin is rejected",
            )
            assert_equal(
                await wire.evaluate(child, "progressClicks"),
                {"type": "number", "value": 1},
                f"{name}: invalid input produces no extra click",
            )


async def _classic_navigation(
    target: WebDriverTarget, fixture: str, results: list[dict[str, Any]], name: str
) -> None:
    with _held_navigation() as (held, arrived, release):
        async with _connection(target, fixture, results, name) as (wire, a, b):
            client = ClassicClient(target.endpoint)

            async def navigate() -> dict[str, Any]:
                try:
                    return await asyncio.to_thread(
                        client.post, f"/session/{wire.session_id}/url", {"url": held}
                    )
                except WebDriverHttpError as error:
                    return {
                        "status": error.response.status,
                        "body": error.response.body,
                    }

            request = asyncio.create_task(navigate())
            try:
                assert_true(
                    await asyncio.to_thread(arrived.wait, 10),
                    f"{name}: Classic navigation started",
                )
                tree = await wire.send("browsingContext.getTree", {"root": b})
                close = await wire.send("browsingContext.close", {"context": a})
                assert_true(
                    not request.done(),
                    f"{name}: Classic navigation is genuinely in flight",
                )
                # ChromeDriver 156 also holds attached BiDi behind this Classic
                # command. Do not invent an early-dispatch requirement here.
                # Both requests must still settle after the real gate opens.
                release.set()
                await wire.response(tree)
                await wire.response(close)
            finally:
                release.set()
                # Closing A can legitimately fail the pending Classic command.
                result = await asyncio.gather(request, return_exceptions=True)
                results.append(
                    {
                        "name": name + "_classic_terminal",
                        "ok": not isinstance(result[0], BaseException),
                        "terminal": str(result[0]),
                    }
                )


async def _input_scope(
    target: WebDriverTarget,
    fixture: str,
    results: list[dict[str, Any]],
    name: str,
    child_first: bool,
) -> None:
    async with _connection(target, fixture, results, name) as (wire, a, _b):
        await wire.evaluate(
            a,
            "new Promise(resolve => { const f=document.createElement('iframe'); f.onload=()=>resolve(true);"
            f"f.src={json.dumps(fixture + '/webdriver/frame')}; document.body.append(f); }})",
        )
        child = (await wire.call("browsingContext.getTree", {"root": a}))["contexts"][
            0
        ]["children"][0]["context"]
        await wire.evaluate(
            child,
            "document.body.innerHTML='<input>'; document.querySelector('input').focus();"
            "globalThis.progressKeys=[]; for(const type of ['keydown','keyup']) addEventListener(type,e=>progressKeys.push([type,e.key,e.shiftKey])); true",
        )
        first, second = (child, a) if child_first else (a, child)
        await wire.call(
            "input.performActions",
            {
                "context": first,
                "actions": [
                    {
                        "type": "key",
                        "id": "scope-key",
                        "actions": [{"type": "keyDown", "value": "\ue008"}],
                    }
                ],
            },
        )
        await wire.call("input.releaseActions", {"context": second})
        keys = await wire.evaluate(child, "JSON.stringify(progressKeys)")
        assert_equal(
            json.loads(keys["value"]),
            [["keydown", "Shift", True], ["keyup", "Shift", False]],
            f"{name}: top-level input state and release",
        )


async def _dialog(
    target: WebDriverTarget,
    fixture: str,
    results: list[dict[str, Any]],
    name: str,
    isolated: bool,
) -> None:
    with _held_navigation() as (start, arrived, release):
        async with _connection(
            target, fixture, results, name, prompt_policy="ignore", isolated=isolated
        ) as (wire, a, b):
            try:
                await wire.call(
                    "session.subscribe",
                    {
                        "events": [
                            "log.entryAdded",
                            "network.responseCompleted",
                            "browsingContext.contextCreated",
                            "browsingContext.userPromptOpened",
                        ],
                        "contexts": [a, b],
                    },
                )
                resource = fixture + "/webdriver/frame?progress-resource"
                await wire.evaluate(
                    b,
                    "void (async () => {"
                    f"await fetch({json.dumps(start)}, {{mode:'no-cors'}}); console.log('progress-console');"
                    "await new Promise(resolve => setTimeout(() => { console.log('progress-timer'); resolve(); }, 0));"
                    f"const f=document.createElement('iframe'); f.src={json.dumps(fixture + '/webdriver/frame?progress-child')}; document.body.append(f);"
                    f"await fetch({json.dumps(resource)}); console.log('progress-done');"
                    "})()",
                )
                assert_true(
                    await asyncio.to_thread(arrived.wait, 10),
                    f"{name}: B awaits the real start gate",
                )
                after = len(wire.messages)
                alert = await wire.send(
                    "script.evaluate",
                    {
                        "target": {"context": a},
                        "expression": "alert('progress-dialog'); 42",
                        "awaitPromise": True,
                    },
                )
                await wire.until(
                    lambda m: m.get("method") == "browsingContext.userPromptOpened"
                    and m["params"]["context"] == a,
                    "A modal prompt",
                    after=after,
                )
                release.set()
                for text in ("progress-console", "progress-timer", "progress-done"):
                    await wire.until(
                        lambda m: m.get("method") == "log.entryAdded"
                        and m["params"].get("text") == text
                        and m["params"]["source"]["context"] == b,
                        text,
                        after=after,
                    )
                await wire.until(
                    lambda m: m.get("method") == "browsingContext.contextCreated"
                    and m["params"].get("parent") == b,
                    "B child created",
                    after=after,
                )
                await wire.until(
                    lambda m: m.get("method") == "network.responseCompleted"
                    and m["params"]["request"]["url"] == resource,
                    "B resource response",
                    after=after,
                )
                assert_true(not wire.has_response(alert), f"{name}: A remains blocked")
                await wire.call(
                    "browsingContext.handleUserPrompt", {"context": a, "accept": True}
                )
                assert_equal(
                    (await wire.response(alert))["result"]["result"],
                    {"type": "number", "value": 42},
                    f"{name}: A resumes after handling",
                )
            finally:
                release.set()


async def run_bidi_progress_group(
    target: WebDriverTarget,
    fixture: str,
    results: list[dict[str, Any]],
    continue_on_failure: bool = False,
) -> None:
    scenarios = [
        (
            "bidi_navigation_interactive_controls",
            lambda name: _navigation(target, fixture, results, name, "interactive"),
        ),
        (
            "bidi_navigation_complete_controls",
            lambda name: _navigation(target, fixture, results, name, "complete"),
        ),
        (
            "bidi_dialog_during_pending_navigation",
            lambda name: _dialog_during_pending_navigation(
                target, fixture, results, name
            ),
        ),
        (
            "bidi_pending_runtime_popup_facts",
            lambda name: _runtime_popup_facts(target, fixture, results, name),
        ),
        (
            "bidi_runtime_independent_pending",
            lambda name: _runtime(target, fixture, results, name),
        ),
        (
            "bidi_input_queue_independence",
            lambda name: _input_queue(target, fixture, results, name),
        ),
        (
            "bidi_runtime_cancel_one_owner",
            lambda name: _runtime_cancel(target, fixture, results, name),
        ),
        (
            "bidi_input_release_during_pause",
            lambda name: _input_release_during_pause(target, fixture, results, name),
        ),
        (
            "bidi_input_release_generations_chrome156",
            lambda name: _input_release_generations(target, fixture, results, name),
        ),
        (
            "bidi_input_pause_controls",
            lambda name: _input_pause(target, fixture, results, name),
        ),
        (
            "bidi_classic_navigation_controls",
            lambda name: _classic_navigation(target, fixture, results, name),
        ),
        (
            "bidi_input_child_top_release",
            lambda name: _input_scope(target, fixture, results, name, True),
        ),
        (
            "bidi_input_top_child_release",
            lambda name: _input_scope(target, fixture, results, name, False),
        ),
        (
            "bidi_dialog_shared_context_output",
            lambda name: _dialog(target, fixture, results, name, False),
        ),
        (
            "bidi_dialog_isolated_context_output",
            lambda name: _dialog(target, fixture, results, name, True),
        ),
    ]
    for name, scenario in scenarios:
        try:
            await scenario(name)
        except Exception as error:
            if not continue_on_failure:
                raise
            record_failure(results, "bidi-progress", name, error)


async def run_bidi_layout_freshness_group(
    target: WebDriverTarget,
    fixture: str,
    results: list[dict[str, Any]],
    continue_on_failure: bool = False,
) -> None:
    """Isolate the existing layout freshness issue without any input in flight."""
    name = "bidi_layout_freshness_without_input"
    try:
        async with _connection(target, fixture, results, name) as (wire, a, _b):
            await wire.evaluate(
                a,
                "document.body.innerHTML='<button style=\"position:absolute;left:20px;top:20px;width:80px;height:60px\">move</button>';true",
            )
            assert_equal(
                await wire.evaluate(
                    a, "document.querySelector('button').getBoundingClientRect().left"
                ),
                {"type": "number", "value": 20},
                f"{name}: actual initial layout",
            )
            await wire.evaluate(
                a,
                "document.querySelector('button').setAttribute('style','position:absolute;left:220px;top:20px;width:80px;height:60px');true",
            )
            await wire.evaluate(
                a, "new Promise(resolve=>requestAnimationFrame(()=>resolve(true)))"
            )
            assert_equal(
                await wire.evaluate(
                    a, "document.querySelector('button').getBoundingClientRect().left"
                ),
                {"type": "number", "value": 220},
                f"{name}: mutation reaches layout without an input command",
            )
    except Exception as error:
        if not continue_on_failure:
            raise
        record_failure(results, "bidi-layout-freshness", name, error)


async def run_bidi_input_layout_group(
    target: WebDriverTarget,
    fixture: str,
    results: list[dict[str, Any]],
    continue_on_failure: bool = False,
) -> None:
    """Requires a layout-enabled endpoint; Mock's explicit error is tested elsewhere."""
    for element in (False, True):
        name = (
            "bidi_input_child_element_layout"
            if element
            else "bidi_input_child_viewport_layout"
        )
        try:
            await _input_child_pointer(target, fixture, results, name, element)
        except Exception as error:
            if not continue_on_failure:
                raise
            record_failure(results, "bidi-input-layout", name, error)


async def run_bidi_auth_progress_group(
    target: WebDriverTarget,
    fixture: str,
    results: list[dict[str, Any]],
    continue_on_failure: bool = False,
) -> None:
    """Diagnostic for Native Fetch multi-target routing, separately from command admission."""
    try:
        await _auth_navigations(
            target, fixture, results, "bidi_concurrent_auth_navigation"
        )
    except Exception as error:
        if not continue_on_failure:
            raise
        record_failure(
            results, "bidi-auth-progress", "bidi_concurrent_auth_navigation", error
        )
