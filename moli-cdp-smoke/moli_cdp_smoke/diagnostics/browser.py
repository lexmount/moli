"""Owned, disposable native-browser CDP sessions for live investigations."""
from __future__ import annotations

import asyncio
from contextlib import asynccontextmanager
from datetime import datetime, timezone
import hashlib
import json
import os
from pathlib import Path
import platform
import socket
import tempfile
import urllib.request

from playwright.async_api import async_playwright

from ..process import subprocess_starts_new_session, terminate_process_tree


def save(path: Path, value: object) -> None:
    path.write_text(json.dumps(value, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")


def require_native_windows(required: bool) -> None:
    if required and platform.system() != "Windows":
        raise ValueError("Windows baseline requires this collector and browser to run on Windows; a UA override is not a Windows baseline")


def discovery(endpoint: str) -> dict:
    opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))
    with opener.open(endpoint + "/json/version", timeout=2) as response:
        return json.load(response)


async def stop_browser(process) -> bool:
    if os.name == "posix":
        return await terminate_process_tree(process)
    # Windows has no SIGKILL/process group API. Browser.close above normally
    # closes the full browser; the owned root process still has a bounded exit.
    if process.returncode is not None:
        return True
    process.terminate()
    try:
        await asyncio.wait_for(process.wait(), 5)
    except asyncio.TimeoutError:
        process.kill()
        await asyncio.wait_for(process.wait(), 2)
    return True


@asynccontextmanager
async def browser_session(binary: Path, engine: str, output: Path, require_windows: bool = False):
    require_native_windows(require_windows)
    binary = binary.resolve(strict=True)
    output.mkdir(parents=True, exist_ok=False)
    with socket.socket() as sock:
        sock.bind(("127.0.0.1", 0))
        port = sock.getsockname()[1]
    endpoint = f"http://127.0.0.1:{port}"
    environment = {k: v for k, v in os.environ.items()
                   if k.lower() not in {"http_proxy", "https_proxy", "all_proxy", "no_proxy"}}
    with tempfile.TemporaryDirectory(prefix="moli-native-baseline-") as profile:
        command = ([str(binary), "serve", "-lr", "--port", str(port)] if engine == "moli" else
                   [str(binary), f"--remote-debugging-port={port}", "--remote-debugging-address=127.0.0.1",
                    f"--user-data-dir={profile}", "--no-first-run", "--no-default-browser-check",
                    "--no-proxy-server", "--password-store=basic", "--window-size=1920,1080", "about:blank"])
        with binary.open("rb") as source:
            digest = hashlib.file_digest(source, "sha256").hexdigest()
        metadata = {"started_at": datetime.now(timezone.utc).isoformat(), "engine": engine,
                    "host_os": platform.system(), "host_release": platform.release(),
                    "machine": platform.machine(), "native_windows_required": require_windows,
                    "binary": str(binary), "binary_sha256": digest,
                    "identity_override": False, "tls_verification": True, "proxy": "disabled",
                    "mode": "CDP; Runtime/Page/Network enabled; fresh process and context; no viewport emulation"}
        if hasattr(os, "getloadavg"):
            metadata["load_average_before"] = os.getloadavg()
        with (output / "server.log").open("w", encoding="utf-8") as log:
            process = await asyncio.create_subprocess_exec(
                *command, env=environment, stdout=log, stderr=asyncio.subprocess.STDOUT,
                start_new_session=subprocess_starts_new_session())
            try:
                for _ in range(60):
                    if process.returncode is not None:
                        raise RuntimeError(f"Browser exited before discovery: {process.returncode}")
                    try:
                        version = await asyncio.to_thread(discovery, endpoint)
                        break
                    except (OSError, ValueError):
                        await asyncio.sleep(0.2)
                else:
                    raise TimeoutError("CDP discovery did not become ready")
                metadata["browser"] = {k: version.get(k) for k in ["Browser", "V8-Version", "User-Agent"]}
                save(output / "environment.json", metadata)
                async with async_playwright() as pw:
                    # Discovery already bypassed proxies; use its WebSocket
                    # directly instead of having the driver repeat the HTTP
                    # request with the caller's proxy environment.
                    browser = await pw.chromium.connect_over_cdp(version["webSocketDebuggerUrl"], timeout=15000)
                    try:
                        context = await asyncio.wait_for(browser.new_context(no_viewport=True), 15)
                        page = await asyncio.wait_for(context.new_page(), 15)
                        cdp = await asyncio.wait_for(context.new_cdp_session(page), 10)
                        for domain in ["Runtime", "Page", "Network"]:
                            await asyncio.wait_for(cdp.send(domain + ".enable"), 10)
                        yield page, cdp
                    finally:
                        try:
                            await asyncio.wait_for(browser.close(), 10)
                        except Exception as error:
                            metadata["close_error_type"] = type(error).__name__
            finally:
                metadata["process_cleaned_up"] = await stop_browser(process)
                metadata["finished_at"] = datetime.now(timezone.utc).isoformat()
                metadata["process_exit"] = process.returncode
                if hasattr(os, "getloadavg"):
                    metadata["load_average_after"] = os.getloadavg()
                save(output / "environment.json", metadata)
