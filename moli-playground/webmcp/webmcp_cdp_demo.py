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
import sys
from datetime import datetime, timedelta, timezone
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
        "--moli-bin", help="Path to a release Moli binary with native WebMCP support"
    )
    parser.add_argument(
        "--cdp-endpoint", help="Reuse a release Moli server, e.g. http://127.0.0.1:9222"
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
