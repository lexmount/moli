#!/usr/bin/env python3
# /// script
# requires-python = ">=3.11"
# dependencies = ["websockets>=15,<18"]
# ///
"""List and call any site's native WebMCP tools, or operate them in one session."""

from __future__ import annotations

import argparse
import asyncio
import json
import os
import stat
import sys
from collections.abc import AsyncIterator
from pathlib import Path
from typing import Any

from webmcp_client import (
    CdpPage,
    DemoError,
    discovery,
    moli_server,
    require,
)
from websockets.asyncio.client import connect
from websockets.exceptions import ConnectionClosed


def arguments(value: str) -> dict[str, Any]:
    try:
        if value.startswith("@"):
            value = Path(value[1:]).read_text(encoding="utf-8")
        elif value == "-":
            value = sys.stdin.read()
        parsed = json.loads(value)
        json.dumps(parsed, allow_nan=False)
    except (OSError, ValueError) as error:
        raise DemoError(f"Invalid tool input: {error}") from error
    require(isinstance(parsed, dict), "Tool input must be a JSON object.")
    return parsed


def emit(value: Any) -> None:
    print(json.dumps(value, ensure_ascii=False, indent=2), flush=True)


def frame_selection(page: CdpPage, frame: str | None) -> str | None:
    return page.frame_id if frame == "main" else None if frame == "all" else frame


def pending(page: CdpPage, invocation_id: str) -> dict[str, Any]:
    step = page.invocation(invocation_id)
    return page.responded.get(invocation_id) or {
        "invocationId": invocation_id,
        "toolName": step["tool"],
        "frameId": step["frameId"],
        "status": "Pending",
    }


HELP = """Commands:
  tools                    List the current tool catalog and schemas
  schema NAME              Show one tool's schema
  frame ID|main|all         Select a frame (default: all)
  call NAME JSON           Start a call; JSON defaults to {}
  result ID                Show the current result without waiting
  wait ID                  Wait for a tool response
  confirm ID               Submit the active declarative form
  cancel ID                Cancel a pending invocation
  help                     Show these commands
  quit                     End the session
JSON may also be @file.json. Calls share the same page state.
"""


async def input_lines() -> AsyncIterator[str]:
    interactive = sys.stdin.isatty()
    encoding = sys.stdin.encoding or "utf-8"
    if stat.S_ISREG(os.fstat(sys.stdin.fileno()).st_mode):
        for line in sys.stdin:
            yield line
        return
    reader = asyncio.StreamReader()
    transport, _ = await asyncio.get_running_loop().connect_read_pipe(
        lambda: asyncio.StreamReaderProtocol(reader), sys.stdin
    )
    try:
        while True:
            if interactive:
                print("webmcp> ", end="", file=sys.stderr, flush=True)
            line = await reader.readline()
            if not line:
                return
            yield line.decode(encoding)
    finally:
        transport.close()


async def shell(page: CdpPage, frame: str | None) -> int:
    print(HELP, file=sys.stderr)
    emit(page.all_tools(frame_selection(page, frame)))
    async for line in input_lines():
        command, _, rest = line.strip().partition(" ")
        if not command:
            continue
        if command in {"quit", "exit"}:
            return 0
        try:
            await page.refresh()
            selected = frame_selection(page, frame)
            if command == "help":
                print(HELP, file=sys.stderr)
            elif command == "tools":
                emit(page.all_tools(selected))
            elif command == "schema":
                emit(page.select_tool(rest.strip(), selected))
            elif command == "frame":
                require(rest.strip(), "Usage: frame ID|main|all")
                frame = rest.strip()
                emit({"frameId": frame_selection(page, frame)})
            elif command == "call":
                name, _, value = rest.strip().partition(" ")
                require(name, "Usage: call NAME JSON")
                tool = page.select_tool(name, selected)
                require(
                    value.strip() != "-", "Use inline JSON or @file.json in the shell."
                )
                invocation_id = await page.begin(
                    name, arguments(value.strip() or "{}"), frame_id=tool["frameId"]
                )
                await page.refresh()
                emit(pending(page, invocation_id))
            elif command in {"result", "wait", "confirm", "cancel"}:
                invocation_id = rest.strip()
                require(invocation_id, f"Usage: {command} ID")
                if command == "result":
                    emit(pending(page, invocation_id))
                elif command == "wait":
                    emit(await page.response(invocation_id))
                elif command == "cancel":
                    emit(await page.cancel(invocation_id))
                else:
                    await page.confirm(invocation_id)
                    await page.refresh()
                    emit(pending(page, invocation_id))
            else:
                raise DemoError(f"Unknown command {command!r}; type help.")
        except DemoError as error:
            print(f"Error: {error}", file=sys.stderr)
    return 0


async def run(args: argparse.Namespace, endpoint: str) -> int:
    version = await asyncio.to_thread(discovery, endpoint)
    websocket_url = version.get("webSocketDebuggerUrl")
    require(websocket_url, "The CDP server did not provide a browser WebSocket URL.")
    async with connect(
        websocket_url, proxy=None, max_size=None, open_timeout=args.timeout
    ) as websocket:
        page = CdpPage(websocket, args.timeout)
        try:
            await page.open(
                None if args.url == "-" else args.url, target_id=args.target_id
            )
            await page.settle(args.wait_for_tools)
            frame = frame_selection(page, args.frame_id)
            if args.action == "list":
                emit(page.all_tools(frame))
                return 0
            if args.action == "shell":
                return await shell(page, args.frame_id)
            tool = page.select_tool(args.tool, frame)
            manual = "backendNodeId" in tool and not tool.get("annotations", {}).get(
                "autosubmit"
            )
            require(
                not manual or args.confirm,
                "This form needs confirmation; use call --confirm, or use shell with call, confirm, and wait.",
            )
            invocation_id = await page.begin(
                args.tool, args.input, frame_id=tool["frameId"]
            )
            if manual:
                await page.until(
                    lambda: (
                        invocation_id in page.invoked or invocation_id in page.responded
                    ),
                    "form filling",
                )
                if invocation_id not in page.responded:
                    await page.confirm(invocation_id)
            response = await page.response(invocation_id)
            emit(response)
            return int(response["status"] != "Completed")
        finally:
            try:
                await page.close()
            except (DemoError, OSError, ConnectionClosed):
                pass


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "url", help="Site URL, or - to keep an existing --target-id page"
    )
    parser.add_argument(
        "action", nargs="?", choices=["list", "call", "shell"], default="list"
    )
    parser.add_argument("tool", nargs="?", help="Tool name for the call action")
    parser.add_argument(
        "--input", default="{}", help="JSON object, @file.json, or - for stdin"
    )
    parser.add_argument(
        "--frame-id", help="Choose a frame, main, or all (default: all)"
    )
    parser.add_argument(
        "--confirm",
        action="store_true",
        help="Confirm a declarative form after filling",
    )
    parser.add_argument(
        "--wait-for-tools",
        type=float,
        default=2,
        help="Seconds to collect initial registrations after page load",
    )
    parser.add_argument(
        "--moli-bin", help="Release Moli binary (default: target/release/moli)"
    )
    parser.add_argument("--cdp-endpoint", help="Reuse an existing release Moli server")
    parser.add_argument(
        "--target-id", help="Attach to a page in --cdp-endpoint; leave it open on exit"
    )
    parser.add_argument(
        "--http-proxy", help="HTTP proxy for a server started by this script"
    )
    parser.add_argument(
        "--timeout", type=float, default=45, help="Command and event timeout in seconds"
    )
    parser.add_argument("--startup-timeout", type=float, default=15)
    args = parser.parse_args()
    if args.action == "call" and not args.tool:
        parser.error("call requires a tool name")
    if args.action != "call" and (args.tool or args.confirm or args.input != "{}"):
        parser.error("tool, --input, and --confirm are only supported with call")
    if args.target_id and not args.cdp_endpoint:
        parser.error("--target-id requires --cdp-endpoint")
    if args.url == "-" and not args.target_id:
        parser.error("URL - requires --target-id")
    if args.timeout <= 0 or args.startup_timeout <= 0 or args.wait_for_tools < 0:
        parser.error(
            "timeouts must be positive and --wait-for-tools must be nonnegative"
        )
    if args.action == "call":
        args.input = arguments(args.input)
    with moli_server(args) as endpoint:
        return asyncio.run(run(args, endpoint))


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except (DemoError, OSError, TimeoutError, ConnectionClosed) as error:
        print(f"WebMCP failed: {error}", file=sys.stderr)
        raise SystemExit(1) from error
    except KeyboardInterrupt:
        raise SystemExit(130)
