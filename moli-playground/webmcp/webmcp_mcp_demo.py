# /// script
# requires-python = ">=3.11"
# dependencies = ["mcp>=1.26,<2", "httpx>=0.28,<1"]
# ///
"""Call a Moli WebMCP service using the official Python MCP client."""

import argparse
import asyncio
import json

import httpx
from mcp import ClientSession
from mcp.client.streamable_http import streamable_http_client


async def run(args: argparse.Namespace) -> None:
    async with (
        httpx.AsyncClient(
            trust_env=False, timeout=httpx.Timeout(30, read=300), follow_redirects=True
        ) as http,
        streamable_http_client(args.endpoint, http_client=http) as (read, write, _),
        ClientSession(read, write) as session,
    ):
        await session.initialize()
        catalog = await session.list_tools()
        if args.list:
            print(catalog.model_dump_json(by_alias=True, exclude_none=True, indent=2))
            return
        available = {tool.name for tool in catalog.tools}
        calls = (
            [(args.tool, json.loads(args.input))]
            if args.tool
            else [
                ("manage_pizza", {"action": "reset"}),
                ("set_pizza_size", {"size": "Large"}),
                ("set_pizza_style", {"style": "BBQ"}),
                ("add_topping", {"topping": "🍍", "size": "Large", "count": 3}),
                ("add_topping", {"topping": "🥓", "size": "Large", "count": 5}),
            ]
        )
        for name, arguments in calls:
            if name not in available:
                raise ValueError(f"Tool {name!r} is unavailable; use --list")
            if not isinstance(arguments, dict):
                raise TypeError("Tool arguments must be a JSON object")
            result = await session.call_tool(name, arguments)
            print(
                json.dumps(
                    {
                        "tool": name,
                        "result": result.model_dump(by_alias=True, exclude_none=True),
                    },
                    ensure_ascii=False,
                ),
                flush=True,
            )
            if result.isError:
                raise RuntimeError(f"Tool {name!r} failed")


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--endpoint", default="http://127.0.0.1:9222/mcp")
    modes = parser.add_mutually_exclusive_group()
    modes.add_argument(
        "--list", action="store_true", help="Print the live tool catalog"
    )
    modes.add_argument("--tool", help="Call a tool instead of running the pizza demo")
    parser.add_argument("--input", default="{}", help="JSON arguments for --tool")
    args = parser.parse_args()
    asyncio.run(run(args))


if __name__ == "__main__":
    main()
