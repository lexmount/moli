"""Native launch profile using the benchmark's existing owned-process manager."""
from __future__ import annotations

import asyncio
from contextlib import asynccontextmanager
from pathlib import Path
import socket

from ..artifacts import write_json
from ..raw_cdp import discover_websocket_url
from ..target_serve import start_target_serve, stop_target_serve


def endpoint_closed(endpoint: str) -> bool:
    port = int(endpoint.rsplit(":", 1)[1])
    with socket.socket() as sock:
        sock.settimeout(1)
        return sock.connect_ex(("127.0.0.1", port)) != 0


@asynccontextmanager
async def browser_session(binary: Path, engine: str, output: Path):
    from playwright.async_api import async_playwright

    output.mkdir(parents=True, exist_ok=False)
    handle = None
    browser = None
    metadata = {"engine": engine, "native": True, "identity_override": False,
                "tls_verification": True, "process_cleaned_up": False}
    try:
        # Startup is bounded by the existing manager. Do not cancel a to_thread
        # launch: its still-running thread could otherwise leave an unowned process.
        target = "moli-full-cdp" if engine == "moli" else "chrome-cdp"
        handle = start_target_serve(target, binary, 15, native=True)
        websocket = await discover_websocket_url(handle.endpoint)
        async with async_playwright() as pw:
            browser = await pw.chromium.connect_over_cdp(websocket, timeout=15000)
            try:
                context = await asyncio.wait_for(browser.new_context(no_viewport=True), 15)
                page = await asyncio.wait_for(context.new_page(), 15)
                cdp = await asyncio.wait_for(context.new_cdp_session(page), 10)
                metadata["browser"] = await asyncio.wait_for(cdp.send("Browser.getVersion"), 10)
                for domain in ["Runtime", "Page", "Network"]:
                    await asyncio.wait_for(cdp.send(domain + ".enable"), 10)
                yield page, cdp
            finally:
                if browser is not None:
                    try:
                        await asyncio.wait_for(browser.close(), 10)
                    except Exception as error:
                        metadata["close_error_type"] = type(error).__name__
    finally:
        if handle is not None:
            stopped = stop_target_serve(handle)
            metadata.update(returncode=stopped["returncode"], endpoint_closed=endpoint_closed(handle.endpoint))
            metadata["process_cleaned_up"] = stopped["process_exited"] and metadata["endpoint_closed"]
        write_json(output / "environment.json", metadata)
