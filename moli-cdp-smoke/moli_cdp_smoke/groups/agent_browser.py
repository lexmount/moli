from __future__ import annotations

import asyncio
import json
import os
import re
import shutil
import tempfile
import urllib.request
import uuid
from pathlib import Path
from typing import Any
from urllib.parse import urlparse

from ..diagnostics import write_artifact
from ..assertions import SmokeError, record
from ..config import clear_proxy_env
from ..process import CapturedProcessTimeout, run_captured_process
from .tracing import _assert_cpu_profile_events, _hot_function_expression


_MIN_AGENT_BROWSER_VERSION = (0, 31, 1)


def _agent_browser_binary() -> str:
    override = os.environ.get("AGENT_BROWSER_BIN")
    if override:
        path = Path(override).expanduser().resolve()
        if not path.is_file():
            raise SmokeError(f"AGENT_BROWSER_BIN does not exist: {path}")
        return str(path)
    binary = shutil.which("agent-browser")
    if binary is None:
        raise SmokeError(
            "agent-browser is not installed; set AGENT_BROWSER_BIN to the 0.31.1+ CLI"
        )
    return binary


def _agent_browser_env() -> dict[str, str]:
    env = clear_proxy_env(os.environ)
    for key in tuple(env):
        if key.startswith("AGENT_BROWSER_"):
            env.pop(key, None)
    env["AGENT_BROWSER_IDLE_TIMEOUT_MS"] = "5000"
    return env


async def _agent_browser_version(binary: str) -> str:
    try:
        process = await run_captured_process(
            [binary, "--version"],
            timeout_seconds=5,
        )
    except CapturedProcessTimeout as error:
        raise SmokeError("agent-browser --version timed out") from error

    output = "\n".join(
        part.decode("utf-8", errors="replace").strip()
        for part in (process.stdout, process.stderr)
        if part
    )
    match = re.search(r"\b(\d+)\.(\d+)\.(\d+)\b", output)
    if process.returncode != 0 or match is None:
        raise SmokeError(f"could not determine agent-browser version: {output!r}")
    version = tuple(int(component) for component in match.groups())
    if version < _MIN_AGENT_BROWSER_VERSION:
        minimum = ".".join(str(component) for component in _MIN_AGENT_BROWSER_VERSION)
        raise SmokeError(f"agent-browser {match.group(0)} is older than required {minimum}")
    return match.group(0)


def _parse_command_payload(stdout: str, stderr: str, argv: list[str]) -> dict[str, Any]:
    candidates = [stdout.strip()]
    candidates.extend(line.strip() for line in reversed(stdout.splitlines()) if line.strip())
    for candidate in candidates:
        if not candidate:
            continue
        try:
            payload = json.loads(candidate)
        except json.JSONDecodeError:
            continue
        if isinstance(payload, dict):
            return payload
    raise SmokeError(
        "agent-browser returned no JSON payload for "
        f"{argv!r}\nstdout:\n{stdout}\nstderr:\n{stderr}"
    )


async def _run_agent_browser(
    binary: str,
    namespace: str,
    session: str,
    config_path: Path,
    cwd: Path,
    *args: str,
    timeout_seconds: float = 15,
) -> dict[str, Any]:
    argv = [
        binary,
        "--namespace",
        namespace,
        "--session",
        session,
        "--config",
        str(config_path),
        "--json",
        *args,
    ]
    try:
        process = await run_captured_process(
            argv,
            timeout_seconds=timeout_seconds,
            cwd=str(cwd),
            env=_agent_browser_env(),
        )
    except CapturedProcessTimeout as error:
        raise SmokeError(
            f"agent-browser command {args!r} timed out after {timeout_seconds}s\n"
            f"stdout:\n{error.stdout.decode(errors='replace')}\n"
            f"stderr:\n{error.stderr.decode(errors='replace')}"
        ) from error

    stdout = process.stdout.decode("utf-8", errors="replace")
    stderr = process.stderr.decode("utf-8", errors="replace")
    return _parse_command_payload(stdout, stderr, argv)


def _require_success(payload: dict[str, Any], label: str) -> dict[str, Any]:
    if payload.get("success") is not True:
        raise SmokeError(f"{label} failed: {payload.get('error')!r}; payload={payload!r}")
    data = payload.get("data")
    if not isinstance(data, dict):
        raise SmokeError(f"{label} returned invalid data: {payload!r}")
    return data


def _read_discovery(endpoint: str) -> dict[str, Any]:
    with urllib.request.urlopen(f"{endpoint.rstrip('/')}/json/version", timeout=5) as response:
        payload = json.load(response)
    if not isinstance(payload, dict):
        raise SmokeError(f"invalid /json/version payload: {payload!r}")
    return payload


async def run_agent_browser_group(
    endpoint: str,
    fixture: str,
    results: list[dict[str, Any]],
) -> None:
    binary = _agent_browser_binary()
    version = await _agent_browser_version(binary)
    discovery = await asyncio.to_thread(_read_discovery, endpoint)
    browser_name = str(discovery.get("Browser", ""))
    browser_websocket = discovery.get("webSocketDebuggerUrl")
    if not isinstance(browser_websocket, str) or not browser_websocket:
        raise SmokeError(f"/json/version has no browser websocket: {discovery!r}")
    is_moli_endpoint = browser_websocket.endswith(
        "/devtools/browser/moli-browser"
    )

    parsed_endpoint = urlparse(endpoint)
    if parsed_endpoint.port is None:
        raise SmokeError(f"agent-browser smoke requires an explicit CDP port: {endpoint}")

    namespace = f"moli-smoke-{uuid.uuid4().hex}"
    session = "goal-path"
    with tempfile.TemporaryDirectory(prefix="moli-agent-browser-smoke-") as temp:
        temp_dir = Path(temp)
        config_path = temp_dir / "agent-browser.json"
        config_path.write_text("{}\n", encoding="ascii")

        async def command(*args: str, timeout_seconds: float = 15) -> dict[str, Any]:
            return await _run_agent_browser(
                binary,
                namespace,
                session,
                config_path,
                temp_dir,
                *args,
                timeout_seconds=timeout_seconds,
            )

        route_installed = False
        try:
            _require_success(await command("connect", str(parsed_endpoint.port)), "connect")
            cdp_url = _require_success(await command("get", "cdp-url"), "get cdp-url").get(
                "cdpUrl"
            )
            if cdp_url != browser_websocket:
                raise SmokeError(
                    "agent-browser attached to the wrong endpoint: "
                    f"expected {browser_websocket!r}, got {cdp_url!r}"
                )
            record(
                results,
                "agent_browser_explicit_cdp_binding",
                {
                    "agentBrowserVersion": version,
                    "browser": browser_name,
                    "webSocketDebuggerUrl": cdp_url,
                },
            )

            page_url = f"{fixture}/plain?client=agent-browser"
            opened = _require_success(await command("open", page_url), "open")
            if opened.get("url") != page_url:
                raise SmokeError(f"agent-browser open returned wrong URL: {opened!r}")
            main_text = _require_success(await command("get", "text", "main"), "get text").get(
                "text"
            )
            if main_text != "plain ok":
                raise SmokeError(f"agent-browser read wrong main text: {main_text!r}")

            setup = """
                document.body.innerHTML = `
                  <input id="name">
                  <input id="keys">
                  <button id="position-click">position click</button>
                `;
                globalThis.__agentBrowserEnterCount = 0;
                globalThis.__agentBrowserClickCount = 0;
                document.querySelector('#keys').addEventListener('keydown', event => {
                  if (event.key === 'Enter') globalThis.__agentBrowserEnterCount += 1;
                });
                document.querySelector('#position-click').addEventListener('click', () => {
                  globalThis.__agentBrowserClickCount += 1;
                });
                'ready';
            """
            setup_result = _require_success(await command("eval", setup), "eval setup").get(
                "result"
            )
            if setup_result != "ready":
                raise SmokeError(f"agent-browser setup returned {setup_result!r}")
            record(results, "agent_browser_open_read_eval_workflow")

            _require_success(await command("fill", "#name", "agent value"), "fill")
            value = _require_success(await command("get", "value", "#name"), "get value").get(
                "value"
            )
            if value != "agent value":
                raise SmokeError(f"agent-browser fill value mismatch: {value!r}")
            _require_success(await command("focus", "#keys"), "focus")
            _require_success(await command("keyboard", "type", "xy"), "keyboard type")
            _require_success(await command("press", "Enter"), "press Enter")
            keyboard_state = _require_success(
                await command(
                    "eval",
                    "({value: document.querySelector('#keys').value, enterCount: __agentBrowserEnterCount})",
                ),
                "eval keyboard state",
            ).get("result")
            if keyboard_state != {"value": "xy", "enterCount": 1}:
                raise SmokeError(f"agent-browser keyboard state mismatch: {keyboard_state!r}")
            record(results, "agent_browser_fill_keyboard_workflow")

            _require_success(await command("set", "media", "dark"), "set media dark")
            dark_matches = _require_success(
                await command("eval", "matchMedia('(prefers-color-scheme: dark)').matches"),
                "eval dark media",
            ).get("result")
            if dark_matches is not True:
                raise SmokeError(f"agent-browser media override did not apply: {dark_matches!r}")
            record(results, "agent_browser_media_workflow")

            route_url = f"{fixture}/agent-browser-route"
            _require_success(
                await command(
                    "network",
                    "route",
                    "**/agent-browser-route",
                    "--body",
                    '{"source":"agent-browser-route"}',
                ),
                "network route",
            )
            route_installed = True
            route_body = _require_success(
                await command("eval", f"fetch('{route_url}').then(response => response.text())"),
                "eval routed fetch",
            ).get("result")
            if route_body != '{"source":"agent-browser-route"}':
                raise SmokeError(f"agent-browser route body mismatch: {route_body!r}")
            request_data = _require_success(
                await command("network", "requests"), "network requests"
            )
            requests = request_data.get("requests")
            if not isinstance(requests, list) or not any(
                isinstance(request, dict) and request.get("url") == route_url
                for request in requests
            ):
                raise SmokeError(f"agent-browser did not record routed request: {request_data!r}")
            _require_success(await command("network", "unroute"), "network unroute")
            route_installed = False
            record(results, "agent_browser_network_route_workflow")

            trace_path = temp_dir / "agent-browser-trace.json"
            _require_success(await command("trace", "start"), "trace start")
            _require_success(
                await command("eval", "globalThis.__agentBrowserTraceWork = 1 + 1"),
                "trace work",
            )
            _require_success(
                await command("trace", "stop", str(trace_path), timeout_seconds=30),
                "trace stop",
            )
            trace_payload = json.loads(trace_path.read_text(encoding="utf-8"))
            write_artifact("agent-browser-trace.json", trace_payload)
            if not isinstance(trace_payload, dict) or not isinstance(
                trace_payload.get("traceEvents"), list
            ):
                raise SmokeError(f"agent-browser trace artifact has wrong shape: {trace_payload!r}")
            record(
                results,
                "agent_browser_tracing_transport_workflow",
                {"traceEventCount": len(trace_payload["traceEvents"])},
            )

            profile_path = temp_dir / "agent-browser-profile.json"
            _require_success(await command("profiler", "start"), "profiler start")
            _require_success(
                await command(
                    "eval",
                    _hot_function_expression("moliAgentBrowserProfilerHotFunction"),
                ),
                "profiler work",
            )
            _require_success(
                await command("profiler", "stop", str(profile_path), timeout_seconds=30),
                "profiler stop",
            )
            profile_payload = json.loads(profile_path.read_text(encoding="utf-8"))
            write_artifact("agent-browser-profile.json", profile_payload)
            if not isinstance(profile_payload, dict) or not isinstance(
                profile_payload.get("traceEvents"), list
            ):
                raise SmokeError(
                    f"agent-browser profiler transport artifact has wrong shape: {profile_payload!r}"
                )
            profile = _assert_cpu_profile_events(
                profile_payload["traceEvents"],
                {"moliAgentBrowserProfilerHotFunction"},
            )
            record(
                results,
                "agent_browser_profiler_transport_workflow",
                {
                    "traceEventCount": len(profile_payload["traceEvents"]),
                    "profileCount": profile.profile_count,
                    "sampleCount": profile.sample_count,
                    "functionSamples": profile.function_samples,
                    "cpuSamplesRequired": True,
                },
            )

            click_payload = await command("click", "#position-click")
            if not is_moli_endpoint and click_payload.get("success") is not True:
                raise SmokeError(
                    "reference Chromium must complete agent-browser position click: "
                    f"{click_payload!r}"
                )
            if click_payload.get("success") is True:
                expected_click_count = 1
                boundary = "layout-supported"
            elif click_payload.get("success") is False and click_payload.get("error"):
                expected_click_count = 0
                boundary = "explicit-client-failure"
            else:
                raise SmokeError(
                    "agent-browser position click must either dispatch the click or fail visibly: "
                    f"{click_payload!r}"
                )
            click_count = _require_success(
                await command("eval", "globalThis.__agentBrowserClickCount"),
                "eval position click count",
            ).get("result")
            if click_count != expected_click_count:
                raise SmokeError(
                    "agent-browser position click mutated the page unexpectedly: "
                    f"expected {expected_click_count}, got {click_count!r}"
                )
            record(
                results,
                "agent_browser_position_click_boundary",
                {"boundary": boundary, "error": click_payload.get("error")},
            )

            tabs_before = _require_success(
                await command("tab", "list"), "tab list before new"
            ).get("tabs")
            if not isinstance(tabs_before, list):
                raise SmokeError(f"agent-browser returned invalid tab list: {tabs_before!r}")
            _require_success(
                await command("tab", "new", "--label", "smoke-child"),
                "tab new",
            )
            tabs_after_new = _require_success(
                await command("tab", "list"), "tab list after new"
            ).get("tabs")
            if not isinstance(tabs_after_new, list) or len(tabs_after_new) != len(tabs_before) + 1:
                raise SmokeError(
                    "agent-browser tab new did not add exactly one target: "
                    f"before={tabs_before!r}; after={tabs_after_new!r}"
                )
            _require_success(
                await command("tab", "close", "smoke-child"),
                "tab close by label",
            )
            tabs_after_close = _require_success(
                await command("tab", "list"), "tab list after close"
            ).get("tabs")
            if not isinstance(tabs_after_close, list) or len(tabs_after_close) != len(tabs_before):
                raise SmokeError(
                    "agent-browser tab close did not restore the target count: "
                    f"before={tabs_before!r}; after={tabs_after_close!r}"
                )
            record(
                results,
                "agent_browser_tab_lifecycle_workflow",
                {"peakTabs": len(tabs_after_new)},
            )
        finally:
            if route_installed:
                try:
                    await command("network", "unroute")
                except Exception:
                    pass
            try:
                await command("close")
            except Exception:
                pass
