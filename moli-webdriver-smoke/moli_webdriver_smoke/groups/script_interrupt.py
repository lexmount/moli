from __future__ import annotations

import time
from typing import Any

from ..assertions import assert_equal, assert_true, record_contract
from ..client import ClassicClient, classic_value
from ..config import WebDriverTarget


def _new_session_payload(target: WebDriverTarget) -> dict[str, Any]:
    always_match: dict[str, Any] = {"browserName": target.browser_name}
    if target.browser_name == "chrome":
        options: dict[str, Any] = {
            "args": [
                "--headless=new",
                "--no-sandbox",
                "--disable-gpu",
                "--disable-dev-shm-usage",
                "--no-proxy-server",
            ]
        }
        if target.browser_binary is not None:
            options["binary"] = str(target.browser_binary)
        always_match["goog:chromeOptions"] = options
    return {"capabilities": {"alwaysMatch": always_match}}


async def run_chromedriver_script_timeout_group(
    target: WebDriverTarget,
    fixture: str,
    results: list[dict[str, Any]],
    _continue_on_failure: bool = False,
) -> None:
    client = ClassicClient(target.endpoint)
    created = client.post("/session", _new_session_payload(target))
    value = classic_value(created)
    session_id = value.get("sessionId")
    assert_true(
        isinstance(session_id, str) and bool(session_id),
        "script-interrupt WebDriver session id",
    )

    yielding_timeout: dict[str, Any] | None = None
    bounded_busy_rounds: list[dict[str, Any]] = []
    try:
        page_url = f"{fixture}/webdriver/basic"
        client.post(f"/session/{session_id}/url", {"url": page_url})
        client.post(f"/session/{session_id}/timeouts", {"script": 150})

        yielding_started = time.monotonic()
        yielding_response = client.request(
            "POST",
            f"/session/{session_id}/execute/async",
            {
                "script": (
                    "const done = arguments[arguments.length - 1];"
                    "setTimeout(() => done('too-late'), 500);"
                ),
                "args": [],
            },
            expected_status=500,
        )
        yielding_elapsed_ms = (time.monotonic() - yielding_started) * 1000.0
        yielding_value = yielding_response.body.get("value")
        assert_true(
            isinstance(yielding_value, dict),
            "WebDriver yielding timeout envelope",
        )
        assert_equal(
            yielding_value.get("error"),
            "script timeout",
            "WebDriver yielding timeout error",
        )
        assert_true(
            100.0 <= yielding_elapsed_ms < 1_000.0,
            f"WebDriver yielding timeout latency: {yielding_elapsed_ms}ms",
        )
        yielding_timeout = {
            "timeoutHttpStatus": yielding_response.status,
            "elapsedMs": round(yielding_elapsed_ms, 1),
        }

        for round_index in range(1, 4):
            busy_ms = 275 + round_index * 25
            started = time.monotonic()
            timeout_response = client.request(
                "POST",
                f"/session/{session_id}/execute/async",
                {
                    "script": (
                        f"globalThis.__webdriverInterruptRound = {round_index};"
                        f"const deadline = performance.now() + {busy_ms};"
                        "while (performance.now() < deadline) {}"
                    ),
                    "args": [],
                },
                expected_status=500,
            )
            elapsed_ms = (time.monotonic() - started) * 1000.0
            timeout_value = timeout_response.body.get("value")
            assert_true(
                isinstance(timeout_value, dict),
                f"WebDriver bounded-busy timeout envelope round {round_index}",
            )
            assert_equal(
                timeout_value.get("error"),
                "script timeout",
                f"WebDriver bounded-busy timeout error round {round_index}",
            )
            assert_true(
                busy_ms - 25.0 <= elapsed_ms < 5_000.0,
                "WebDriver script timeout must wait for non-yielding JavaScript to return "
                f"in round {round_index}: busy={busy_ms}ms elapsed={elapsed_ms}ms",
            )

            recovered = client.post(
                f"/session/{session_id}/execute/sync",
                {
                    "script": (
                        "return {"
                        "  marker: globalThis.__webdriverInterruptRound,"
                        f"  recovery: {round_index} * 42"
                        "};"
                    ),
                    "args": [],
                },
            )
            assert_equal(
                classic_value(recovered),
                {"marker": round_index, "recovery": round_index * 42},
                f"WebDriver isolate recovery round {round_index}",
            )
            callback = client.post(
                f"/session/{session_id}/execute/async",
                {
                    "script": (
                        "arguments[arguments.length - 1]("
                        f"'recovered-{round_index}'"
                        ");"
                    ),
                    "args": [],
                },
            )
            assert_equal(
                classic_value(callback),
                f"recovered-{round_index}",
                f"WebDriver async callback recovery round {round_index}",
            )
            bounded_busy_rounds.append(
                {
                    "round": round_index,
                    "busyMs": busy_ms,
                    "timeoutHttpStatus": timeout_response.status,
                    "elapsedMs": round(elapsed_ms, 1),
                }
            )

        record_contract(
            results,
            "webdriver_classic_script_timeout_yield_boundary",
            contract=(
                "ChromeDriver settles a yielding async script at the configured timeout, but "
                "cannot preempt JavaScript that is currently running: a bounded busy loop first "
                "returns to the renderer, then reports script timeout. The same window remains "
                "usable for repeated sync and async commands."
            ),
            source=(
                "ChromeDriver 147 executable oracle for the WebDriver Classic script-timeout "
                "yield boundary"
            ),
            commands=[
                "POST /session/{sessionId}/timeouts",
                "POST /session/{sessionId}/execute/async (yielding timeout)",
                "POST /session/{sessionId}/execute/async (bounded busy loop) x3",
                "POST /session/{sessionId}/execute/sync x3",
                "POST /session/{sessionId}/execute/async (callback) x3",
            ],
            observed={
                "yieldingTimeout": yielding_timeout,
                "boundedBusyRounds": bounded_busy_rounds,
            },
        )
    finally:
        try:
            client.post(f"/session/{session_id}/timeouts", {"script": 30_000})
        finally:
            client.delete(f"/session/{session_id}")


async def run_moli_script_interrupt_group(
    target: WebDriverTarget,
    fixture: str,
    results: list[dict[str, Any]],
    _continue_on_failure: bool = False,
) -> None:
    client = ClassicClient(target.endpoint)
    created = client.post("/session", _new_session_payload(target))
    value = classic_value(created)
    session_id = value.get("sessionId")
    assert_true(
        isinstance(session_id, str) and bool(session_id),
        "script-interrupt WebDriver session id",
    )

    rounds: list[dict[str, Any]] = []
    try:
        client.post(f"/session/{session_id}/url", {"url": f"{fixture}/webdriver/basic"})
        client.post(f"/session/{session_id}/timeouts", {"script": 150})

        for round_index, endpoint_name in enumerate(
            ("execute/async", "execute/sync", "execute/async"),
            start=1,
        ):
            marker = f"interrupt-{round_index}"
            script = (
                f"globalThis.__webdriverInterruptMarker = '{marker}';"
                "for (;;) {}"
            )
            started = time.monotonic()
            timeout_response = client.request(
                "POST",
                f"/session/{session_id}/{endpoint_name}",
                {"script": script, "args": []},
                expected_status=500,
            )
            elapsed_ms = (time.monotonic() - started) * 1000.0
            timeout_value = timeout_response.body.get("value")
            assert_true(
                isinstance(timeout_value, dict),
                f"WebDriver interrupt timeout envelope round {round_index}",
            )
            assert_equal(
                timeout_value.get("error"),
                "script timeout",
                f"WebDriver interrupt timeout error round {round_index}",
            )
            assert_true(
                100.0 <= elapsed_ms < 2_000.0,
                "WebDriver IO termination must preempt non-yielding JavaScript near the "
                f"configured timeout in round {round_index}: elapsed={elapsed_ms}ms",
            )

            recovered = client.post(
                f"/session/{session_id}/execute/sync",
                {
                    "script": (
                        "return {"
                        "  marker: globalThis.__webdriverInterruptMarker,"
                        f"  recovery: {round_index} * 42"
                        "};"
                    ),
                    "args": [],
                },
            )
            assert_equal(
                classic_value(recovered),
                {"marker": marker, "recovery": round_index * 42},
                f"WebDriver isolate recovery after IO termination round {round_index}",
            )
            callback = client.post(
                f"/session/{session_id}/execute/async",
                {
                    "script": (
                        "arguments[arguments.length - 1]("
                        f"'recovered-{round_index}'"
                        ");"
                    ),
                    "args": [],
                },
            )
            assert_equal(
                classic_value(callback),
                f"recovered-{round_index}",
                f"WebDriver async callback after IO termination round {round_index}",
            )
            rounds.append(
                {
                    "round": round_index,
                    "endpoint": endpoint_name,
                    "timeoutHttpStatus": timeout_response.status,
                    "elapsedMs": round(elapsed_ms, 1),
                }
            )

        record_contract(
            results,
            "webdriver_classic_non_yielding_script_io_termination",
            contract=(
                "Moli's WebDriver Classic script timeout uses the renderer IO lane to "
                "terminate non-yielding sync and async JavaScript. Each timeout settles "
                "once near its deadline, and the same window immediately accepts sync and "
                "async script work after every termination."
            ),
            source=(
                "Moli WebDriver extension over Chromium's CDP "
                "Runtime.terminateExecution IO boundary"
            ),
            commands=[
                "POST /session/{sessionId}/timeouts",
                "POST /session/{sessionId}/execute/async (infinite loop) x2",
                "POST /session/{sessionId}/execute/sync (infinite loop)",
                "POST /session/{sessionId}/execute/sync (recovery) x3",
                "POST /session/{sessionId}/execute/async (callback recovery) x3",
            ],
            observed={"terminationRounds": rounds},
        )
    finally:
        try:
            client.post(f"/session/{session_id}/timeouts", {"script": 30_000})
        finally:
            client.delete(f"/session/{session_id}")
