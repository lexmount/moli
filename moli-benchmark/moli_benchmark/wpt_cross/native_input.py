"""Native WPT automation on a separate CDP connection from harness probes.

The input connection must remain runnable while the harness connection awaits a
JavaScript promise. Each instance belongs to one case target and is disposed
with it; no pressed keys, pending bindings, or pointer state cross case borders.
Permission requests use frames exposed by the attached page session; out-of-
process iframe targets require a separate attachment and report an error here.
"""

from __future__ import annotations

import asyncio
import contextlib
import json
import math
import time
from dataclasses import dataclass, field
from typing import Any
from urllib.parse import urlsplit

from ..raw_cdp import (
    RawCdpConnectionClosed,
    RawCdpError,
    RawCdpTimeoutError,
    RoutedRawCdpClient,
    connect_routed_raw_cdp,
)

BINDING_NAME = "__bench_wpt_native_input__"
STATE_NAME = "__bench_wpt_native_input_state__"


@dataclass(frozen=True)
class Key:
    key: str
    code: str
    virtual_code: int
    location: int = 0
    text: str = ""


_SPECIAL_KEYS = {
    "\ue001": Key("Cancel", "Cancel", 3),
    "\ue002": Key("Help", "Help", 47),
    "\ue003": Key("Backspace", "Backspace", 8),
    "\ue004": Key("Tab", "Tab", 9),
    "\ue005": Key("Clear", "Numpad5", 12, 3),
    "\ue006": Key("Enter", "Enter", 13, text="\r"),
    "\ue007": Key("Enter", "NumpadEnter", 13, 3, "\r"),
    "\ue008": Key("Shift", "ShiftLeft", 16, 1),
    "\ue009": Key("Control", "ControlLeft", 17, 1),
    "\ue00a": Key("Alt", "AltLeft", 18, 1),
    "\ue00b": Key("Pause", "Pause", 19),
    "\ue00c": Key("Escape", "Escape", 27),
    "\ue00d": Key(" ", "Space", 32, text=" "),
    "\ue010": Key("End", "End", 35),
    "\ue011": Key("Home", "Home", 36),
    "\ue00e": Key("PageUp", "PageUp", 33),
    "\ue00f": Key("PageDown", "PageDown", 34),
    "\ue012": Key("ArrowLeft", "ArrowLeft", 37),
    "\ue013": Key("ArrowUp", "ArrowUp", 38),
    "\ue014": Key("ArrowRight", "ArrowRight", 39),
    "\ue015": Key("ArrowDown", "ArrowDown", 40),
    "\ue016": Key("Insert", "Insert", 45),
    "\ue017": Key("Delete", "Delete", 46),
    "\ue03d": Key("Meta", "MetaLeft", 91, 1),
    "\ue050": Key("Shift", "ShiftRight", 16, 2),
    "\ue051": Key("Control", "ControlRight", 17, 2),
    "\ue052": Key("Alt", "AltRight", 18, 2),
    "\ue053": Key("Meta", "MetaRight", 92, 2),
    "\n": Key("Enter", "Enter", 13, text="\r"),
    "\r": Key("Enter", "Enter", 13, text="\r"),
    "\t": Key("Tab", "Tab", 9),
}
_SPECIAL_KEYS.update({chr(0xE01A + n): Key(str(n), f"Numpad{n}", 96 + n, 3, str(n)) for n in range(10)})
_SPECIAL_KEYS.update({chr(0xE031 + n): Key(f"F{n + 1}", f"F{n + 1}", 112 + n) for n in range(12)})
_SPECIAL_KEYS.update({
    chr(0xE024 + n): Key(char, "Numpad" + name, 106 + n, 3, char)
    for n, (char, name) in enumerate(zip("*+,-./", ("Multiply", "Add", "Comma", "Subtract", "Decimal", "Divide")))
})
_MODIFIERS = {"Alt": 1, "Control": 2, "Meta": 4, "Shift": 8}
_PUNCTUATION = {
    " ": ("Space", 32), "`": ("Backquote", 192), "-": ("Minus", 189),
    "=": ("Equal", 187), "[": ("BracketLeft", 219), "]": ("BracketRight", 221),
    "\\": ("Backslash", 220), ";": ("Semicolon", 186), "'": ("Quote", 222),
    ",": ("Comma", 188), ".": ("Period", 190), "/": ("Slash", 191),
}
_SHIFTED = dict(zip("`1234567890-=[]\\;',./", '~!@#$%^&*()_+{}|:"<>?'))
_UNSHIFTED = {value: key for key, value in _SHIFTED.items()}


def key_description(value: str, *, shift: bool = False) -> Key:
    if value in _SPECIAL_KEYS:
        return _SPECIAL_KEYS[value]
    value = {"\ue018": ";", "\ue019": "="}.get(value, value)
    if not isinstance(value, str) or len(value) != 1 or 0xE000 <= ord(value) <= 0xF8FF:
        raise ValueError(f"unsupported WebDriver key: {value!r}")
    base = _UNSHIFTED.get(value, value).lower()
    text = value
    if shift:
        text = _SHIFTED.get(value, value.upper() if value.isascii() else value)
    if base.isascii() and base.isalpha():
        code, virtual_code = "Key" + base.upper(), ord(base.upper())
    elif base.isascii() and base.isdigit():
        code, virtual_code = "Digit" + base, ord(base)
    else:
        code, virtual_code = _PUNCTUATION.get(base, ("", 0))
    if not code:
        raise ValueError("native WPT input requires an ASCII or WebDriver special key")
    return Key(text, code, virtual_code, text=text)


@dataclass
class Pointer:
    kind: str = "mouse"
    x: float = 0
    y: float = 0
    buttons: list[int] = field(default_factory=list)
    click_count: int = 0
    last_click: tuple[float, float, float, int] | None = None


def validate_actions(sources: Any) -> None:
    if not isinstance(sources, list):
        raise ValueError("actions must be a list")
    ids: set[str] = set()
    allowed = {
        "none": {"pause"}, "key": {"pause", "keyDown", "keyUp"},
        "pointer": {"pause", "pointerMove", "pointerDown", "pointerUp"},
        "wheel": {"pause", "scroll"},
    }
    for source in sources:
        if not isinstance(source, dict) or source.get("type") not in allowed:
            raise ValueError("unsupported input source")
        source_id = source.get("id")
        if not isinstance(source_id, str) or source_id in ids:
            raise ValueError("input source IDs must be unique strings")
        ids.add(source_id)
        actions = source.get("actions")
        if not isinstance(actions, list):
            raise ValueError("source actions must be a list")
        if source["type"] == "pointer":
            parameters = source.get("parameters", {})
            if not isinstance(parameters, dict) or parameters.get("pointerType", "mouse") not in {"mouse", "pen"}:
                raise ValueError("native WPT input supports mouse and pen pointer sources")
        for action in actions:
            if not isinstance(action, dict) or action.get("type") not in allowed[source["type"]]:
                raise ValueError("unsupported input action")
            kind = action["type"]
            if "duration" in action and (type(action["duration"]) is not int or action["duration"] < 0):
                raise ValueError("action duration must be a nonnegative integer")
            if kind in {"keyDown", "keyUp"}:
                key_description(action.get("value"))
            if kind in {"pointerDown", "pointerUp"} and (
                type(action.get("button")) is not int or not 0 <= action["button"] <= 4
            ):
                raise ValueError("unsupported pointer button")
            if kind in {"pointerMove", "scroll"}:
                fields = ("x", "y", "deltaX", "deltaY") if kind == "scroll" else ("x", "y")
                if any(type(action.get(name)) is not int for name in fields):
                    raise ValueError("action coordinates and deltas must be integers")
                origin = action.get("origin", "viewport")
                if isinstance(origin, dict):
                    if type(origin.get("element")) is not int or origin["element"] < 0:
                        raise ValueError("invalid element origin")
                elif origin != "viewport" and not (kind == "pointerMove" and origin == "pointer"):
                    raise ValueError("unsupported input origin")
            for name, low, high in (
                ("pressure", 0, 1), ("tangentialPressure", -1, 1),
                ("tiltX", -90, 90), ("tiltY", -90, 90), ("twist", 0, 359),
            ):
                if name in action:
                    value = action[name]
                    if type(value) not in (int, float) or not math.isfinite(value) or not low <= value <= high:
                        raise ValueError(f"invalid pointer {name}")
            if any(name in action for name in ("width", "height", "altitudeAngle", "azimuthAngle")):
                raise ValueError("native pointer geometry properties are not supported")


def permission_origin(frame: dict[str, Any]) -> str:
    # Frame.securityOrigin preserves inherited about:blank origins and opaque
    # sandbox origins, which cannot be reconstructed from location.href.
    origin = frame.get("securityOrigin", frame.get("url"))
    if not isinstance(origin, str):
        raise ValueError("permission frame has no origin")
    parsed = urlsplit(origin)
    if parsed.scheme not in {"http", "https"} or not parsed.hostname or parsed.username or parsed.password:
        raise ValueError("cannot set permission for an opaque or unsupported origin")
    return f"{parsed.scheme}://{parsed.netloc}"


class NativeInput:
    def __init__(
        self, client: RoutedRawCdpClient, session_id: str,
        browser_context_id: str | None = None,
    ) -> None:
        self.client = client
        self.session_id = session_id
        self.browser_context_id = browser_context_id
        self.deadline: float | None = None
        self.task: asyncio.Task[None] | None = None
        self.keyboards: dict[str, dict[str, Key]] = {}
        self.pointers: dict[str, Pointer] = {}
        self._closed = False
        self._permissions_changed = False
        self._permission_cleanup_error: Exception | None = None

    @classmethod
    async def attach(
        cls, endpoint: str, target_id: str, browser_context_id: str | None = None,
    ) -> NativeInput:
        client = await connect_routed_raw_cdp(endpoint)
        try:
            result = await client.command("Target.attachToTarget", {"targetId": target_id, "flatten": True})
            instance = cls(client, result.response["result"]["sessionId"], browser_context_id)
            await instance.command("Runtime.enable")
            await instance.command("Runtime.addBinding", {"name": BINDING_NAME})
            instance.task = asyncio.create_task(instance.run(), name="wpt-native-input")
            return instance
        except BaseException:
            await client.close()
            raise

    async def close(self) -> None:
        if self._closed:
            if self._permission_cleanup_error is not None:
                raise RawCdpError("failed to reset WPT permissions") from self._permission_cleanup_error
            return
        self._closed = True
        if self.task is not None:
            self.task.cancel()
            with contextlib.suppress(asyncio.CancelledError, Exception):
                await self.task
        if self._permissions_changed:
            params = {"browserContextId": self.browser_context_id} if self.browser_context_id else {}
            try:
                await self.client.command("Browser.resetPermissions", params, timeout=5)
            except Exception as error:
                self._permission_cleanup_error = error
        # The engine can exit before cleanup. Still detach when possible and
        # always close our own receiver and socket, even after a send fails.
        with contextlib.suppress(Exception):
            await self.client.command(
                "Runtime.removeBinding", {"name": BINDING_NAME},
                session_id=self.session_id, timeout=5,
            )
        with contextlib.suppress(Exception):
            await self.client.command(
                "Target.detachFromTarget", {"sessionId": self.session_id}, timeout=5,
            )
        await self.client.close()
        if self._permission_cleanup_error is not None:
            raise RawCdpError("failed to reset WPT permissions") from self._permission_cleanup_error

    async def command(self, method: str, params: dict[str, Any] | None = None) -> dict[str, Any]:
        timeout = 10.0 if self.deadline is None else max(0.01, self.deadline - time.perf_counter())
        result = await self.client.command(method, params, session_id=self.session_id, timeout=timeout)
        return result.response.get("result", {})

    async def evaluate(self, context_id: int, expression: str) -> Any:
        result = await self.command("Runtime.evaluate", {
            "contextId": context_id, "expression": expression, "returnByValue": True,
        })
        if result.get("exceptionDetails"):
            details = result["exceptionDetails"]
            raise ValueError((details.get("exception") or {}).get("description") or details.get("text"))
        return result.get("result", {}).get("value")

    async def run(self) -> None:
        sequence = 0
        while True:
            try:
                event = await self.client.wait_for_event(
                    "Runtime.bindingCalled", after_sequence=sequence,
                    session_id=self.session_id,
                    predicate=lambda event: event.get("params", {}).get("name") == BINDING_NAME,
                    timeout=60,
                )
            except RawCdpTimeoutError:
                continue
            except RawCdpConnectionClosed:
                return
            sequence = event.sequence
            params = event.payload["params"]
            try:
                request = json.loads(params["payload"])
                request_id = request["id"]
                token = request["token"]
                if not isinstance(request_id, int) or not isinstance(token, str):
                    continue
            except (ValueError, KeyError, TypeError):
                continue
            context_id = params["executionContextId"]
            error = None
            try:
                # This lookup also rejects a request whose originating realm or
                # pending promise was destroyed before its turn reached us.
                await self.evaluate(context_id, f"{STATE_NAME}.check({request_id}, {json.dumps(token)})")
                await self.perform(context_id, request)
            except Exception as exc:
                error = str(exc)
            try:
                await self.evaluate(
                    context_id,
                    f"{STATE_NAME}.finish({request_id}, {json.dumps(token)}, {json.dumps(error)})",
                )
            except (RawCdpError, ValueError):
                # Navigation can destroy a request's realm during native input.
                # Never deliver its reply to the new document's similarly named promise.
                pass

    def modifiers(self) -> int:
        result = 0
        for keyboard in self.keyboards.values():
            for key in keyboard.values():
                result |= _MODIFIERS.get(key.key, 0)
        return result

    async def key(self, source: str, value: str, down: bool) -> None:
        keyboard = self.keyboards.setdefault(source, {})
        if not down and value not in keyboard:
            return
        repeat = value in keyboard
        if down:
            key = key_description(value, shift=bool(self.modifiers() & 8))
            keyboard[value] = key
        else:
            key = keyboard.pop(value)
        modifiers = self.modifiers()
        text = key.text if down and not modifiers & 7 else ""
        params: dict[str, Any] = {
            "type": "keyUp" if not down else "keyDown" if text else "rawKeyDown",
            "key": key.key, "code": key.code, "windowsVirtualKeyCode": key.virtual_code,
            "modifiers": modifiers, "autoRepeat": down and repeat,
            "location": key.location if key.location != 3 else 0,
            "isKeypad": key.location == 3,
        }
        if text:
            params.update(text=text, unmodifiedText=text)
        await self.command("Input.dispatchKeyEvent", params)

    async def release_keyboard(self, source: str) -> None:
        for value in reversed(list(self.keyboards.get(source, {}))):
            await self.key(source, value, False)
        self.keyboards.pop(source, None)

    async def send_keys(self, keys: str) -> None:
        source = "__send_keys__"
        try:
            for value in keys:
                if value == "\ue000":
                    await self.release_keyboard(source)
                    continue
                key = key_description(value)
                if key.key in _MODIFIERS:
                    if value not in self.keyboards.get(source, {}):
                        await self.key(source, value, True)
                    continue
                shift = (value.isascii() and value.isupper()) or value in _UNSHIFTED
                own_shift = "\ue008" in self.keyboards.get(source, {})
                if shift and not own_shift:
                    await self.key(source, "\ue008", True)
                elif not shift and own_shift:
                    await self.key(source, "\ue008", False)
                await self.key(source, value, True)
                await self.key(source, value, False)
        finally:
            await self.release_keyboard(source)

    async def mouse(self, pointer: Pointer, event: str, button: int | None = None, **extra: Any) -> None:
        button_names = {0: "left", 1: "middle", 2: "right", 3: "back", 4: "forward"}
        button_bits = {0: 1, 1: 4, 2: 2, 3: 8, 4: 16}
        if button is not None and button not in button_names:
            raise ValueError(f"unsupported mouse button: {button}")
        if event == "mousePressed":
            if button in pointer.buttons:
                return
            pointer.buttons.append(button)
            now = time.perf_counter()
            last = pointer.last_click
            repeated_click = (
                last and now - last[0] < 0.5
                and (pointer.x, pointer.y, button) == last[1:]
            )
            pointer.click_count = pointer.click_count + 1 if repeated_click else 1
            pointer.last_click = (now, pointer.x, pointer.y, button)
        elif event == "mouseReleased":
            if button not in pointer.buttons:
                return
            pointer.buttons.remove(button)
        await self.command("Input.dispatchMouseEvent", {
            "type": event, "x": pointer.x, "y": pointer.y,
            "button": button_names.get(button, "none"),
            "buttons": sum(button_bits[b] for b in pointer.buttons),
            "modifiers": self.modifiers(), "pointerType": pointer.kind,
            "clickCount": pointer.click_count if event in {"mousePressed", "mouseReleased"} else 0,
            "force": 0.5 if pointer.buttons else 0,
            **extra,
        })

    async def reset_actions(self) -> None:
        for source in reversed(list(self.keyboards)):
            await self.release_keyboard(source)
        for pointer in self.pointers.values():
            for button in reversed(list(pointer.buttons)):
                await self.mouse(pointer, "mouseReleased", button)
        self.pointers.clear()

    async def point(
        self, context_id: int, request_id: int, token: str, origin: Any,
        x: float, y: float,
    ) -> tuple[float, float]:
        value = await self.evaluate(
            context_id,
            f"{STATE_NAME}.point({request_id}, {json.dumps(token)}, {json.dumps(origin)}, {x}, {y})",
        )
        if (
            not isinstance(value, list) or len(value) != 2
            or not all(isinstance(n, (int, float)) and math.isfinite(n) for n in value)
        ):
            raise ValueError("invalid input coordinates")
        return value[0], value[1]

    async def perform(self, context_id: int, request: dict[str, Any]) -> None:
        request_id = request["id"]
        token = request["token"]
        kind = request["kind"]
        if kind == "set_permission":
            await self.set_permission(context_id, request)
        elif kind == "send_keys":
            await self.evaluate(context_id, f"{STATE_NAME}.focus({request_id}, {json.dumps(token)})")
            await self.send_keys(request["keys"])
        elif kind == "click":
            x, y = await self.point(context_id, request_id, token, "viewport", request["x"], request["y"])
            pointer = self.pointers.setdefault("__click__", Pointer())
            pointer.x, pointer.y = x, y
            await self.mouse(pointer, "mouseMoved")
            await self.mouse(pointer, "mousePressed", 0)
            await self.mouse(pointer, "mouseReleased", 0)
        elif kind == "actions":
            sources = request["actions"]
            validate_actions(sources)
            await self.reset_actions()
            for source in sources:
                if source["type"] == "pointer":
                    pointer_type = source.get("parameters", {}).get("pointerType", "mouse")
                    if pointer_type not in {"mouse", "pen"}:
                        raise ValueError(f"native WPT pointer type is not supported: {pointer_type}")
                    self.pointers[source["id"]] = Pointer(kind=pointer_type)
                elif source["type"] not in {"key", "none", "wheel"}:
                    raise ValueError(f"unsupported input source: {source['type']}")
            ticks = max((len(source["actions"]) for source in sources), default=0)
            for index in range(ticks):
                actions = [(source, source["actions"][index]) for source in sources if index < len(source["actions"])]
                duration = max((action.get("duration", 0) for _, action in actions), default=0) / 1000
                started = time.perf_counter()
                tasks = [
                    asyncio.create_task(self.action(context_id, request_id, token, source, action))
                    for source, action in actions
                ]
                try:
                    await asyncio.gather(*tasks)
                finally:
                    # A failed or canceled tick must not leave another source
                    # dispatching input after this request has ended.
                    for task in tasks:
                        task.cancel()
                    await asyncio.gather(*tasks, return_exceptions=True)
                await asyncio.sleep(max(0, duration - (time.perf_counter() - started)))
        else:
            raise ValueError(f"unsupported native input request: {kind}")

    async def set_permission(self, context_id: int, request: dict[str, Any]) -> None:
        descriptor = request.get("descriptor")
        state = request.get("state")
        if not isinstance(descriptor, dict) or not isinstance(descriptor.get("name"), str) or not descriptor["name"]:
            raise ValueError("permission descriptor requires a name")
        if not isinstance(state, str) or state not in {"granted", "denied", "prompt"}:
            raise ValueError("invalid permission state")
        path = await self.evaluate(
            context_id,
            f"{STATE_NAME}.permissionFramePath({request['id']}, {json.dumps(request['token'])})",
        )
        if not isinstance(path, list) or any(type(index) is not int or index < 0 for index in path):
            raise ValueError("invalid permission frame path")
        tree = (await self.command("Page.getFrameTree")).get("frameTree")
        if not isinstance(tree, dict) or not isinstance(tree.get("frame"), dict):
            raise ValueError("permission target has no frame tree")
        embedding_origin = permission_origin(tree["frame"])
        for index in path:
            children = tree.get("childFrames", [])
            if not isinstance(children, list) or index >= len(children):
                raise ValueError("permission frame is unavailable in this CDP session (detached or out of process)")
            tree = children[index]
        embedded_origin = permission_origin(tree["frame"])
        if path:
            # A session's frame tree can omit out-of-process frames. Verify the
            # owner instead of letting an omitted sibling shift the indices.
            owner = await self.command("DOM.getFrameOwner", {"frameId": tree["frame"]["id"]})
            resolved = await self.command("DOM.resolveNode", {
                "backendNodeId": owner["backendNodeId"], "executionContextId": context_id,
            })
            object_id = resolved["object"]["objectId"]
            try:
                matches = await self.command("Runtime.callFunctionOn", {
                    "executionContextId": context_id,
                    "functionDeclaration": (
                        "function(owner, id, token) { return "
                        f"{STATE_NAME}.permissionFrameMatches(id, token, owner); }}"
                    ),
                    "arguments": [
                        {"objectId": object_id}, {"value": request["id"]}, {"value": request["token"]},
                    ],
                    "returnByValue": True,
                })
                if matches.get("exceptionDetails") or matches.get("result", {}).get("value") is not True:
                    raise ValueError("permission frame does not match the requested Window")
            finally:
                await self.command("Runtime.releaseObject", {"objectId": object_id})
        # Resolve the retained Window again after the protocol read, so a
        # detached or moved frame cannot grant a sibling's origin by index.
        current_path = await self.evaluate(
            context_id,
            f"{STATE_NAME}.permissionFramePath({request['id']}, {json.dumps(request['token'])})",
        )
        if current_path != path:
            raise ValueError("permission frame changed during the request")
        params = {
            "permission": descriptor, "setting": state,
            "origin": embedding_origin, "embeddedOrigin": embedded_origin,
        }
        if self.browser_context_id is not None:
            params["browserContextId"] = self.browser_context_id
        # A command can be applied even if its response is lost. Always reset
        # after attempting a mutation, and never attach a page sessionId to it.
        self._permissions_changed = True
        timeout = 10.0 if self.deadline is None else max(0.01, self.deadline - time.perf_counter())
        await self.client.command("Browser.setPermission", params, timeout=timeout)

    async def action(
        self, context_id: int, request_id: int, token: str,
        source: dict[str, Any], action: dict[str, Any],
    ) -> None:
        kind = action["type"]
        if kind == "pause":
            return
        if source["type"] == "key" and kind in {"keyDown", "keyUp"}:
            await self.key(source["id"], action["value"], kind == "keyDown")
            return
        if source["type"] == "wheel" and kind == "scroll":
            x, y = await self.point(
                context_id, request_id, token,
                action.get("origin", "viewport"), action["x"], action["y"],
            )
            await self.mouse(Pointer(x=x, y=y), "mouseWheel", deltaX=action["deltaX"], deltaY=action["deltaY"])
            return
        if source["type"] != "pointer":
            raise ValueError(f"unsupported {source['type']} action: {kind}")
        pointer = self.pointers[source["id"]]
        extra = {name: action[name] for name in ("tiltX", "tiltY", "twist", "tangentialPressure") if name in action}
        if "pressure" in action:
            extra["force"] = action["pressure"]
        if kind in {"pointerDown", "pointerUp"}:
            await self.mouse(
                pointer, "mousePressed" if kind == "pointerDown" else "mouseReleased",
                action["button"], **extra,
            )
        elif kind == "pointerMove":
            origin = action.get("origin", "viewport")
            if origin == "pointer":
                x, y = await self.point(
                    context_id, request_id, token, origin,
                    pointer.x + action["x"], pointer.y + action["y"],
                )
            else:
                x, y = await self.point(context_id, request_id, token, origin, action["x"], action["y"])
            duration = action.get("duration", 0) / 1000
            steps = max(1, math.ceil(duration / 0.016))
            start_x, start_y = pointer.x, pointer.y
            started = time.perf_counter()
            for step in range(1, steps + 1):
                await asyncio.sleep(max(0, started + duration * step / steps - time.perf_counter()))
                pointer.x = start_x + (x - start_x) * step / steps
                pointer.y = start_y + (y - start_y) * step / steps
                await self.mouse(pointer, "mouseMoved", **extra)
        else:
            raise ValueError(f"unsupported pointer action: {kind}")
