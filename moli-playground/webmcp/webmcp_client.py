"""Shared release Moli server and native WebMCP CDP client."""

from __future__ import annotations

import argparse
import asyncio
import json
import socket
import subprocess
import sys
import tempfile
import time
import urllib.request
from collections import deque
from collections.abc import Callable
from contextlib import contextmanager, suppress
from pathlib import Path
from typing import Any

try:
    from websockets.asyncio.client import ClientConnection
    from websockets.exceptions import ConnectionClosed
except ImportError as error:
    raise SystemExit(
        "Run with `uv run webmcp.py` or `uv run webmcp_cdp_demo.py`."
    ) from error


REPO_ROOT = Path(__file__).resolve().parents[2]


class DemoError(RuntimeError):
    pass


def require(condition: Any, message: str) -> None:
    if not condition:
        raise DemoError(message)


def discovery(endpoint: str) -> dict[str, Any]:
    # CDP discovery must reach the local server even when an HTTP proxy is set.
    opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))
    with opener.open(endpoint.rstrip("/") + "/json/version", timeout=2) as response:
        return json.load(response)


@contextmanager
def moli_server(args: argparse.Namespace):
    if args.cdp_endpoint:
        yield args.cdp_endpoint
        return
    if args.moli_bin:
        binary = Path(args.moli_bin).expanduser().resolve()
    else:
        binary = (REPO_ROOT / "target" / "release" / "moli").resolve()
    require(
        "debug" not in binary.parts,
        "Debug builds are not allowed; use a release Moli binary.",
    )
    require(
        binary.is_file(),
        "Build Moli first: `cargo build --release -p moli`, or pass --moli-bin with a release binary.",
    )
    with socket.socket() as sock:
        sock.bind(("127.0.0.1", 0))
        port = sock.getsockname()[1]
    endpoint = f"http://127.0.0.1:{port}"
    with (
        tempfile.TemporaryDirectory(prefix="moli-webmcp-") as directory,
        tempfile.TemporaryFile(mode="w+t") as logs,
    ):
        command = [
            str(binary),
            "serve",
            "--host",
            "127.0.0.1",
            "--port",
            str(port),
            "--profile-dir",
            str(Path(directory) / "profile"),
            "--http-timeout",
            str(int(args.timeout * 1000)),
        ]
        if args.http_proxy:
            command.extend(["--http-proxy", args.http_proxy])
        process = subprocess.Popen(command, cwd=REPO_ROOT, stdout=logs, stderr=logs)
        try:
            deadline = time.monotonic() + args.startup_timeout
            while time.monotonic() < deadline and process.poll() is None:
                try:
                    discovery(endpoint)
                    break
                except (OSError, ValueError):
                    time.sleep(0.05)
            else:
                logs.seek(0)
                raise DemoError(f"Moli did not start:\n{logs.read()[-4000:]}")
            print(f"Moli: {binary}\nCDP: {endpoint}", file=sys.stderr)
            yield endpoint
        finally:
            if process.poll() is None:
                process.terminate()
                try:
                    process.wait(timeout=3)
                except subprocess.TimeoutExpired:
                    process.kill()
                    process.wait(timeout=3)


class CdpPage:
    def __init__(self, websocket: ClientConnection, timeout: float):
        self.websocket = websocket
        self.timeout = timeout
        self.next_id = 1
        self.session_id: str | None = None
        self.target_id: str | None = None
        self.frame_id: str | None = None
        self.dom_ready = False
        self.tools: dict[tuple[str, str], dict[str, Any]] = {}
        self.invoked: set[str] = set()
        self.responded: dict[str, dict[str, Any]] = {}
        self.errors: deque[dict[str, Any]] = deque(maxlen=5)
        self.steps: list[dict[str, Any]] = []
        self.replies: dict[int, asyncio.Future[dict[str, Any]]] = {}
        self.changed = asyncio.Event()
        self.reader_error: Exception | None = None
        self.reader = asyncio.create_task(self.read_messages())

    async def read_messages(self) -> None:
        try:
            while True:
                message = await self.receive()
                reply = self.replies.get(message.get("id"))
                if reply is not None and not reply.done():
                    reply.set_result(message)
                self.changed.set()
        except (OSError, ValueError, ConnectionClosed) as error:
            self.reader_error = error
            for reply in self.replies.values():
                if not reply.done():
                    reply.set_exception(error)
            self.changed.set()

    async def receive(self) -> dict[str, Any]:
        message = json.loads(await self.websocket.recv())
        if message.get("sessionId") != self.session_id:
            return message
        method = message.get("method")
        params = message.get("params", {})
        if method == "WebMCP.toolsAdded":
            for tool in params["tools"]:
                self.tools[(tool["frameId"], tool["name"])] = tool
        elif method == "WebMCP.toolsRemoved":
            for tool in params["tools"]:
                self.tools.pop((tool["frameId"], tool["name"]), None)
        elif method == "WebMCP.toolInvoked":
            self.invoked.add(params["invocationId"])
        elif method == "WebMCP.toolResponded":
            # A result may arrive before the invokeTool command reply.
            self.responded[params["invocationId"]] = params
        elif method == "Page.domContentEventFired":
            self.dom_ready = True
        elif method == "Runtime.executionContextsCleared":
            self.tools.clear()
            self.dom_ready = False
        elif method == "Page.frameNavigated" and not params["frame"].get("parentId"):
            self.frame_id = params["frame"]["id"]
        elif method == "Page.frameDetached":
            self.tools = {
                key: tool
                for key, tool in self.tools.items()
                if key[0] != params["frameId"]
            }
        elif method == "Runtime.exceptionThrown":
            self.errors.append(params.get("exceptionDetails", params))
        return message

    async def command(
        self,
        method: str,
        params: dict[str, Any] | None = None,
        *,
        browser: bool = False,
    ) -> dict[str, Any]:
        message_id = self.next_id
        self.next_id += 1
        message: dict[str, Any] = {
            "id": message_id,
            "method": method,
            "params": params or {},
        }
        if self.session_id and not browser:
            message["sessionId"] = self.session_id
        if self.reader_error is not None:
            raise self.reader_error
        reply = asyncio.get_running_loop().create_future()
        self.replies[message_id] = reply
        try:
            async with asyncio.timeout(self.timeout):
                await self.websocket.send(json.dumps(message, ensure_ascii=False))
                response = await reply
                if "error" in response:
                    raise DemoError(f"{method}: {response['error']}")
                return response.get("result", {})
        except TimeoutError as error:
            raise DemoError(f"Timed out waiting for {method}") from error
        finally:
            self.replies.pop(message_id, None)
            if not reply.done():
                reply.cancel()
            elif not reply.cancelled():
                reply.exception()

    async def until(self, predicate: Callable[[], bool], label: str) -> None:
        try:
            async with asyncio.timeout(self.timeout):
                while not predicate():
                    if self.reader_error is not None:
                        raise self.reader_error
                    self.changed.clear()
                    await self.changed.wait()
        except TimeoutError as error:
            raise DemoError(
                f"Timed out waiting for {label}; page errors: {list(self.errors)}"
            ) from error

    async def evaluate(self, expression: str) -> Any:
        response = await self.command(
            "Runtime.evaluate",
            {
                "expression": expression,
                "returnByValue": True,
            },
        )
        if "exceptionDetails" in response:
            raise DemoError(f"Runtime.evaluate: {response['exceptionDetails']}")
        return response.get("result", {}).get("value")

    async def open(
        self,
        url: str | None,
        expected: set[str] | None = None,
        *,
        target_id: str | None = None,
    ) -> None:
        self.owns_target = target_id is None
        if target_id is None:
            target = await self.command(
                "Target.createTarget", {"url": "about:blank"}, browser=True
            )
            target_id = target["targetId"]
        self.target_id = target_id
        attached = await self.command(
            "Target.attachToTarget",
            {
                "targetId": self.target_id,
                "flatten": True,
            },
            browser=True,
        )
        self.session_id = attached["sessionId"]
        for method in ("Page.enable", "Runtime.enable", "WebMCP.enable"):
            await self.command(method)
        if url is not None:
            self.dom_ready = False
            navigation = await self.command("Page.navigate", {"url": url})
            require(not navigation.get("errorText"), f"Navigation failed: {navigation}")
            self.frame_id = navigation["frameId"]
            await self.until(lambda: self.dom_ready, "DOMContentLoaded")
        else:
            await self.refresh()
        if expected is not None:
            require(
                await self.evaluate(
                    "typeof ModelContext !== 'undefined' && document.modelContext instanceof ModelContext"
                ),
                "This page does not expose the native ModelContext API.",
            )
            await self.until(
                lambda: (
                    expected.issubset(self.catalog())
                    if expected
                    else bool(self.catalog())
                ),
                f"native tools {sorted(expected)}",
            )

    async def settle(self, seconds: float) -> None:
        await asyncio.sleep(seconds)
        await self.refresh()

    async def refresh(self) -> None:
        # A command reply is a barrier for already queued tool-change events.
        tree = await self.command("Page.getFrameTree")
        self.frame_id = tree["frameTree"]["frame"]["id"]

    def all_tools(self, frame_id: str | None = None) -> list[dict[str, Any]]:
        return [
            tool
            for (frame, _), tool in sorted(self.tools.items())
            if frame_id is None or frame == frame_id
        ]

    def select_tool(self, name: str, frame_id: str | None = None) -> dict[str, Any]:
        matches = [tool for tool in self.all_tools(frame_id) if tool["name"] == name]
        require(
            matches,
            f"Native tool {name!r} is not registered; list tools to see the current catalog.",
        )
        require(
            len(matches) == 1,
            f"Tool {name!r} exists in multiple frames: {[tool['frameId'] for tool in matches]}; select --frame-id or use the shell's frame command.",
        )
        return matches[0]

    def catalog(self) -> dict[str, dict[str, Any]]:
        return {
            name: tool
            for (frame, name), tool in self.tools.items()
            if frame == self.frame_id
        }

    async def begin(
        self, name: str, arguments: dict[str, Any], *, frame_id: str | None = None
    ) -> str:
        tool = self.tools.get((frame_id or self.frame_id, name))
        require(tool, f"Native tool {name!r} is not registered.")
        print(f"  invoke {name} (frame {tool['frameId']})", file=sys.stderr)
        response = await self.command(
            "WebMCP.invokeTool",
            {
                "frameId": tool["frameId"],
                "toolName": name,
                "input": arguments,
            },
        )
        invocation_id = response["invocationId"]
        step = {
            "tool": name,
            "frameId": tool["frameId"],
            "input": arguments,
            "invocationId": invocation_id,
        }
        if "backendNodeId" in tool:
            step["backendNodeId"] = tool["backendNodeId"]
        self.steps.append(step)
        return invocation_id

    def invocation(self, invocation_id: str) -> dict[str, Any]:
        step = next(
            (step for step in self.steps if step["invocationId"] == invocation_id), None
        )
        require(step, f"Unknown invocation {invocation_id!r} in this session.")
        return step

    async def response(self, invocation_id: str) -> dict[str, Any]:
        step = self.invocation(invocation_id)
        await self.until(
            lambda: invocation_id in self.responded, f"tool response {invocation_id}"
        )
        response = self.responded[invocation_id]
        self.invoked.discard(invocation_id)
        step["response"] = response
        return response

    async def finish(self, invocation_id: str) -> Any:
        response = await self.response(invocation_id)
        require(
            response.get("status") == "Completed", f"Tool invocation failed: {response}"
        )
        return response.get("output")

    async def invoke(self, name: str, arguments: dict[str, Any]) -> Any:
        return await self.finish(await self.begin(name, arguments))

    async def confirm(self, invocation_id: str) -> None:
        step = self.invocation(invocation_id)
        require(
            "backendNodeId" in step,
            "This invocation does not belong to a declarative form.",
        )
        await self.until(
            lambda: invocation_id in self.invoked or invocation_id in self.responded,
            f"form invocation {invocation_id}",
        )
        require(
            invocation_id not in self.responded, "This invocation has already finished."
        )
        node = await self.command(
            "DOM.resolveNode", {"backendNodeId": step["backendNodeId"]}
        )
        object_id = node["object"]["objectId"]
        try:
            result = await self.command(
                "Runtime.callFunctionOn",
                {
                    "objectId": object_id,
                    "functionDeclaration": """function() {
                    if (!this.matches(':tool-form-active')) throw new Error('The tool form is no longer active.');
                    const button = Array.from(this.elements).find(el => el.matches(':tool-submit-active'));
                    if (button) {
                        if (button.matches(':disabled')) throw new Error('The submit button is disabled.');
                        button.click();
                    } else this.requestSubmit();
                }""",
                },
            )
            require(
                "exceptionDetails" not in result, f"Form confirmation failed: {result}"
            )
        finally:
            try:
                await self.command("Runtime.releaseObject", {"objectId": object_id})
            except (DemoError, OSError, ConnectionClosed):
                pass

    async def cancel(self, invocation_id: str) -> dict[str, Any]:
        self.invocation(invocation_id)
        await self.command("WebMCP.cancelInvocation", {"invocationId": invocation_id})
        return await self.response(invocation_id)

    async def close(self) -> None:
        try:
            if self.target_id and not self.owns_target:
                for step in self.steps:
                    if step["invocationId"] not in self.responded:
                        with suppress(DemoError):
                            await self.cancel(step["invocationId"])
            if self.target_id and self.owns_target:
                await self.command(
                    "Target.closeTarget", {"targetId": self.target_id}, browser=True
                )
            elif self.session_id:
                await self.command(
                    "Target.detachFromTarget",
                    {"sessionId": self.session_id},
                    browser=True,
                )
        finally:
            self.reader.cancel()
            with suppress(asyncio.CancelledError):
                await self.reader
