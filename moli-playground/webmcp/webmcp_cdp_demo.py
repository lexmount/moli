#!/usr/bin/env python3
# /// script
# requires-python = ">=3.11"
# dependencies = ["websockets>=15,<18"]
# ///
"""Discover and invoke native WebMCP tools on public demo sites using Moli CDP."""

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
from contextlib import contextmanager
from datetime import datetime, timedelta, timezone
from pathlib import Path
from typing import Any

try:
    from websockets.asyncio.client import ClientConnection, connect
    from websockets.exceptions import ConnectionClosed
except ImportError as error:
    raise SystemExit("Run this script with `uv run webmcp_cdp_demo.py`.") from error


REPO_ROOT = Path(__file__).resolve().parents[2]
DEMO_BASE = "https://googlechromelabs.github.io/webmcp-tools/demos/"
DEMOS = {
    "booking": ("explainer/", {"getAvailability", "bookSlot", "cancelBooking"}),
    "bistro": ("french-bistro/", {"book_table_le_petit_bistro"}),
    "pizza": (
        "pizza-maker/",
        {
            "set_pizza_size",
            "set_pizza_style",
            "toggle_layer",
            "add_topping",
            "remove_topping",
            "manage_pizza",
            "share_pizza",
        },
    ),
}


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
        candidates = [
            REPO_ROOT / "target" / mode / "moli" for mode in ("debug", "release")
        ]
        available = [path for path in candidates if path.is_file()]
        require(
            available, "Build Moli first: `cargo build -p moli`, or pass --moli-bin."
        )
        binary = max(available, key=lambda path: path.stat().st_mtime)
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
        await self.websocket.send(json.dumps(message, ensure_ascii=False))
        try:
            async with asyncio.timeout(self.timeout):
                while True:
                    response = await self.receive()
                    if response.get("id") == message_id:
                        if "error" in response:
                            raise DemoError(f"{method}: {response['error']}")
                        return response.get("result", {})
        except TimeoutError as error:
            raise DemoError(f"Timed out waiting for {method}") from error

    async def until(self, predicate: Callable[[], bool], label: str) -> None:
        try:
            async with asyncio.timeout(self.timeout):
                while not predicate():
                    await self.receive()
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

    async def open(self, url: str, expected: set[str]) -> None:
        target = await self.command(
            "Target.createTarget", {"url": "about:blank"}, browser=True
        )
        self.target_id = target["targetId"]
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
        self.dom_ready = False
        navigation = await self.command("Page.navigate", {"url": url})
        require(not navigation.get("errorText"), f"Navigation failed: {navigation}")
        self.frame_id = navigation["frameId"]
        await self.until(lambda: self.dom_ready, "DOMContentLoaded")
        require(
            await self.evaluate(
                "typeof ModelContext !== 'undefined' && document.modelContext instanceof ModelContext"
            ),
            "This page does not expose the native ModelContext API.",
        )
        await self.until(
            lambda: (
                expected.issubset(self.catalog()) if expected else bool(self.catalog())
            ),
            f"native tools {sorted(expected)} (received: {sorted(self.catalog())})",
        )

    def catalog(self) -> dict[str, dict[str, Any]]:
        return {
            name: tool
            for (frame, name), tool in self.tools.items()
            if frame == self.frame_id
        }

    async def begin(self, name: str, arguments: dict[str, Any]) -> str:
        tool = self.catalog().get(name)
        require(tool, f"Native tool {name!r} is not registered.")
        print(
            f"  invoke {name} {json.dumps(arguments, ensure_ascii=False)}",
            file=sys.stderr,
        )
        response = await self.command(
            "WebMCP.invokeTool",
            {
                "frameId": tool["frameId"],
                "toolName": name,
                "input": arguments,
            },
        )
        invocation_id = response["invocationId"]
        self.steps.append(
            {"tool": name, "input": arguments, "invocationId": invocation_id}
        )
        return invocation_id

    async def finish(self, invocation_id: str) -> Any:
        await self.until(
            lambda: invocation_id in self.responded, f"tool response {invocation_id}"
        )
        response = self.responded.pop(invocation_id)
        self.invoked.discard(invocation_id)
        self.steps[-1]["response"] = response
        require(
            response.get("status") == "Completed", f"Tool invocation failed: {response}"
        )
        return response.get("output")

    async def invoke(self, name: str, arguments: dict[str, Any]) -> Any:
        return await self.finish(await self.begin(name, arguments))

    async def close(self) -> None:
        if self.target_id:
            await self.command(
                "Target.closeTarget", {"targetId": self.target_id}, browser=True
            )


async def booking(page: CdpPage) -> dict[str, Any]:
    today = datetime.now(timezone.utc).date()
    availability = await page.invoke(
        "getAvailability",
        {
            "startDate": today.isoformat(),
            "endDate": (today + timedelta(days=30)).isoformat(),
        },
    )
    require(
        isinstance(availability, dict), f"Unexpected availability: {availability!r}"
    )
    slots = [(day, times[0]) for day, times in sorted(availability.items()) if times]
    require(slots, "The demo returned no available consultation times.")
    day, slot = slots[0]
    reservation = await page.invoke(
        "bookSlot",
        {
            "date": day,
            "time": slot,
            "name": "Moli Playground",
            "email": "demo@example.com",
        },
    )
    require(
        isinstance(reservation, dict) and reservation.get("ok"),
        f"Booking failed: {reservation}",
    )
    widget_state = """(() => {
        const widget = document.querySelector('#widget-with');
        const confirmation = widget.querySelector('.w-success');
        return {
            confirmationVisible: confirmation.classList.contains('is-visible'),
            confirmationText: confirmation.textContent,
            name: widget.querySelector('input[name="name"]').value,
            email: widget.querySelector('input[name="email"]').value,
            submitDisabled: widget.querySelector('.w-confirm').disabled
        };
    })()"""
    booked = await page.evaluate(widget_state)
    require(
        booked["confirmationVisible"]
        and reservation["confirmationId"] in booked["confirmationText"],
        f"Booking did not reach the widget: {booked}",
    )
    cancellation = await page.invoke(
        "cancelBooking", {"confirmationId": reservation["confirmationId"]}
    )
    require(
        isinstance(cancellation, dict)
        and cancellation.get("ok")
        and cancellation.get("cancelled") == reservation["confirmationId"],
        f"Cancellation failed: {cancellation}",
    )
    cancelled = await page.evaluate(widget_state)
    require(
        not cancelled["confirmationVisible"]
        and cancelled["submitDisabled"]
        and cancelled["name"] == cancelled["email"] == "",
        f"The cancelled booking was not reset: {cancelled}",
    )
    return {
        "reservation": reservation,
        "booked_widget": booked,
        "cancellation": cancellation,
        "cancelled_widget": cancelled,
    }


async def bistro(page: CdpPage) -> dict[str, Any]:
    day = (datetime.now(timezone.utc).date() + timedelta(days=7)).isoformat()
    arguments = {
        "name": "Moli Playground",
        "phone": "1234567890",
        "date": day,
        "time": "19:00",
        "guests": "2",
        "seating": "Terrace",
        "requests": "Window seat, please.",
    }
    invocation_id = await page.begin("book_table_le_petit_bistro", arguments)
    await page.until(lambda: invocation_id in page.invoked, "native form filling")
    filled = await page.evaluate("""(() => {
        const form = document.querySelector('#reservationForm');
        return {active: form.matches(':tool-form-active'),
            submitterFocused: document.activeElement === document.querySelector('#submitBtn'),
            values: Object.fromEntries(Array.from(form.elements).filter(el => el.name)
                .map(el => [el.name, el.value]))};
    })()""")
    require(
        filled["active"] and filled["submitterFocused"],
        f"Manual confirmation was not prepared: {filled}",
    )
    require(
        all(filled["values"].get(key) == value for key, value in arguments.items()),
        f"The native tool did not fill the requested values: {filled}",
    )
    require(
        invocation_id not in page.responded, "The form completed before confirmation."
    )
    print("  form filled; confirm through its regular submit button", file=sys.stderr)
    await page.evaluate("document.querySelector('#submitBtn').click()")
    result = await page.finish(invocation_id)
    state = await page.evaluate("""({
        dialogOpen: document.querySelector('#bookingDialog').open,
        details: document.querySelector('#modalDetails').textContent,
        active: document.querySelector('#reservationForm').matches(':tool-form-active')
    })""")
    require(
        state["dialogOpen"] and not state["active"],
        f"Unexpected confirmation state: {state}",
    )
    require(
        isinstance(result, str) and "Moli Playground" in result and "Terrace" in result,
        f"Unexpected form result: {result!r}",
    )
    return {
        "before_confirmation": filled,
        "result": result,
        "after_confirmation": state,
    }


async def pizza(page: CdpPage) -> dict[str, Any]:
    await page.invoke("manage_pizza", {"action": "reset"})
    await page.invoke("set_pizza_size", {"size": "Large"})
    await page.invoke("set_pizza_style", {"style": "BBQ"})
    await page.invoke("add_topping", {"topping": "🍍", "size": "Large", "count": 3})
    await page.invoke("add_topping", {"topping": "🥓", "size": "Large", "count": 5})
    state = await page.evaluate("""({
        size: document.querySelector('#size-text').textContent,
        sauce: document.documentElement.style.getPropertyValue('--sauce'),
        toppings: Array.from(document.querySelectorAll('#pizza-container .topping'),
            node => ({emoji: node.dataset.emoji, size: node.dataset.size}))
    })""")
    require(
        state["size"] == "Large" and state["sauce"] == "#5d4037",
        f"Pizza settings did not update: {state}",
    )
    require(
        len(state["toppings"]) == 8
        and all(item["size"] == "Large" for item in state["toppings"]),
        f"Pizza toppings did not update: {state}",
    )
    require(
        [item["emoji"] for item in state["toppings"]].count("🍍") == 3
        and [item["emoji"] for item in state["toppings"]].count("🥓") == 5,
        f"The requested toppings are missing: {state}",
    )
    return state


FLOWS = {"booking": booking, "bistro": bistro, "pizza": pizza}


async def run(args: argparse.Namespace, endpoint: str) -> list[dict[str, Any]]:
    version = await asyncio.to_thread(discovery, endpoint)
    websocket_url = version.get("webSocketDebuggerUrl")
    require(websocket_url, f"Missing browser WebSocket URL: {version}")
    names = (
        ["inspect"] if args.url else list(DEMOS) if args.demo == "all" else [args.demo]
    )
    reports = []
    for name in names:
        path, expected = DEMOS[name] if name != "inspect" else ("", set())
        url = args.url or DEMO_BASE + path
        report: dict[str, Any] = {
            "demo": name,
            "url": url,
            "browser": version.get("Browser"),
        }
        print(f"[{name}] {url}", file=sys.stderr)
        async with connect(
            websocket_url, proxy=None, max_size=None, open_timeout=args.timeout
        ) as websocket:
            page = CdpPage(websocket, args.timeout)
            try:
                await page.open(url, expected)
                report["tools"] = list(page.catalog().values())
                print(
                    f"  native tools: {', '.join(sorted(page.catalog()))}",
                    file=sys.stderr,
                )
                if not args.list_tools:
                    report["result"] = await FLOWS[name](page)
                report["status"] = "passed"
            except (DemoError, OSError, TimeoutError, ConnectionClosed) as error:
                report.update(
                    status="failed", error=str(error), page_errors=list(page.errors)
                )
                print(f"  FAILED: {error}", file=sys.stderr)
            finally:
                report["steps"] = page.steps
                try:
                    await page.close()
                except (DemoError, OSError, TimeoutError, ConnectionClosed):
                    pass
        reports.append(report)
    return reports


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--demo", choices=["all", *DEMOS], default="all")
    parser.add_argument(
        "--list-tools",
        action="store_true",
        help="Only discover tools and their schemas",
    )
    parser.add_argument(
        "--url", help="Inspect another public URL; requires --list-tools"
    )
    parser.add_argument(
        "--moli-bin", help="Path to a Moli binary with native WebMCP support"
    )
    parser.add_argument(
        "--cdp-endpoint", help="Reuse an existing server, e.g. http://127.0.0.1:9222"
    )
    parser.add_argument("--http-proxy", help="HTTP proxy used by the Moli process")
    parser.add_argument(
        "--timeout", type=float, default=45, help="Command and event timeout in seconds"
    )
    parser.add_argument("--startup-timeout", type=float, default=15)
    parser.add_argument(
        "--output", type=Path, help="Also write the JSON report to this file"
    )
    args = parser.parse_args()
    if args.url and not args.list_tools:
        parser.error("--url requires --list-tools")
    if args.timeout <= 0 or args.startup_timeout <= 0:
        parser.error("timeouts must be positive")
    with moli_server(args) as endpoint:
        reports = asyncio.run(run(args, endpoint))
    payload = json.dumps(reports, ensure_ascii=False, indent=2)
    if args.output:
        args.output.parent.mkdir(parents=True, exist_ok=True)
        args.output.write_text(payload + "\n", encoding="utf-8")
    print(payload)
    return int(any(report["status"] != "passed" for report in reports))


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except (DemoError, OSError, TimeoutError, ConnectionClosed) as error:
        print(f"WebMCP demo failed: {error}", file=sys.stderr)
        raise SystemExit(1) from error
    except KeyboardInterrupt:
        raise SystemExit(130)
