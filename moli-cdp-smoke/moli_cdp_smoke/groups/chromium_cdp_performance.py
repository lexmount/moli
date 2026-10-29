from __future__ import annotations
from typing import Any

from ..diagnostics import new_cdp_session
from . import SmokeState
from ..config import is_moli_browser
from ..assertions import SmokeError, assert_equal, wait_until
from ..helpers import attach_cdp_event_collector

from .chromium_cdp_support import _assert_coverage_contains_function, _assert_profile_tree_shape, _assert_script_coverage_array, _coverage_function_total_count, _events_with_method, _find_coverage_function, _find_script_coverage_by_url, _has_event, _navigate_with_cdp_until_dom_ready, _performance_metrics, _profile_function_names, _send_cdp_expect_optional_error


async def _verify_chromium_performance_enable_sample(state: SmokeState) -> None:
    async def expect_error(method: str, params: dict[str, Any], message: str) -> None:
        error = await _send_cdp_expect_optional_error(state.cdp, method, params)
        if not error or message not in str(error):
            raise SmokeError(f"{method} should fail with {message!r}: {error}")

    for params in (
        {},
        {"timeDomain": "timeTicks"},
        {"timeDomain": "threadTicks"},
        {"timeDomain": None},
    ):
        await state.cdp.send("Performance.enable", params)
        await state.cdp.send("Performance.disable")

    await expect_error(
        "Performance.enable",
        {"timeDomain": "bogusTicks"},
        "Invalid time domain specification.",
    )
    await expect_error(
        "Performance.enable",
        {"timeDomain": "TimeTicks"},
        "Invalid time domain specification.",
    )
    await expect_error(
        "Performance.enable",
        {"timeDomain": 1},
        "Invalid parameters",
    )

    await state.cdp.send("Performance.enable", {"timeDomain": "threadTicks"})
    await state.cdp.send("Performance.enable", {"timeDomain": "threadTicks"})
    await expect_error(
        "Performance.enable",
        {},
        "Cannot change time domain while performance metrics collection is enabled.",
    )
    await expect_error(
        "Performance.setTimeDomain",
        {"timeDomain": "timeTicks"},
        "Cannot set time domain while performance metrics collection is enabled.",
    )
    await state.cdp.send("Performance.disable")
    await state.cdp.send("Performance.disable")

    await state.cdp.send("Performance.setTimeDomain", {"timeDomain": "threadTicks"})
    await expect_error(
        "Performance.setTimeDomain",
        {"timeDomain": "bogusTicks"},
        "Invalid time domain specification.",
    )
    await state.cdp.send("Performance.enable", {"timeDomain": None})
    await state.cdp.send("Performance.disable")

    attached = await new_cdp_session(state.context, state.page)
    try:
        primary_disabled = await _performance_metrics(state.cdp)
        attached_disabled = await _performance_metrics(attached)
        if primary_disabled or attached_disabled:
            raise SmokeError(
                "Performance.getMetrics should be empty before each Inspector session is enabled"
            )

        await state.cdp.send("Performance.enable")
        if await _performance_metrics(attached):
            raise SmokeError("Performance.enable must not enable an attached Inspector session")
        await attached.send("Performance.enable", {"timeDomain": "threadTicks"})
        if not await _performance_metrics(state.cdp) or not await _performance_metrics(attached):
            raise SmokeError("enabled Performance sessions should each expose metrics")
        await state.cdp.send("Performance.disable")
        if not await _performance_metrics(attached):
            raise SmokeError("disabling one Performance session must not disable another")
        await attached.send("Performance.disable")
    finally:
        await attached.detach()
    state.record("chromium_performance_enable_sample")

async def _verify_chromium_performance_metrics_sample(state: SmokeState) -> None:
    await _navigate_with_cdp_until_dom_ready(state, f"{state.fixture}/chromium-cdp-dom-page")
    before_enable = await _performance_metrics(state.cdp)
    await state.cdp.send("Performance.enable")
    enabled_metrics = await _performance_metrics(state.cdp)
    await state.page.evaluate("() => { for (let i = 0; i < 1000; i += 1) Math.sqrt(i); }")
    after_work = await _performance_metrics(state.cdp)
    await state.cdp.send("Performance.disable")
    after_disable = await _performance_metrics(state.cdp)

    required = {
        "Timestamp",
        "Documents",
        "Frames",
        "Nodes",
        "LayoutCount",
        "RecalcStyleCount",
        "LayoutDuration",
        "RecalcStyleDuration",
        "ScriptDuration",
        "TaskDuration",
        "JSHeapUsedSize",
        "JSHeapTotalSize",
    }
    if before_enable:
        raise SmokeError(f"Performance.getMetrics before enable should be empty: {before_enable}")
    if after_disable:
        raise SmokeError(f"Performance.getMetrics after disable should be empty: {after_disable}")
    for label, metrics in [
        ("after enable", enabled_metrics),
        ("after work", after_work),
    ]:
        missing = required.difference(metrics)
        if missing:
            raise SmokeError(f"Performance.getMetrics {label} missing metrics: {sorted(missing)}")
        for name in required:
            value = metrics[name]
            if not isinstance(value, (int, float)) or value < 0:
                raise SmokeError(f"Performance.getMetrics {label} metric {name} invalid: {value!r}")
    if after_work["Timestamp"] < enabled_metrics["Timestamp"]:
        raise SmokeError(f"Performance Timestamp should be monotonic: {enabled_metrics['Timestamp']} -> {after_work['Timestamp']}")
    state.record("chromium_performance_metrics_sample")

async def _verify_cpu_throttling_capability_multiple_pages_sample(state: SmokeState) -> None:
    await _navigate_with_cdp_until_dom_ready(state, f"{state.fixture}/plain")
    second_page = await state.context.new_page()
    second_cdp = await new_cdp_session(state.context, second_page)
    try:
        await second_page.goto(
            f"{state.fixture}/plain?cpu-throttle-second",
            wait_until="load",
            timeout=10_000,
        )
        # Moli supports disabling throttling, but cannot throttle execution.
        # A successful no-op here would mislead clients about the CPU rate.
        for cdp, rate in [(state.cdp, 2.0), (second_cdp, 3.0)]:
            error = await _send_cdp_expect_optional_error(
                cdp, "Emulation.setCPUThrottlingRate", {"rate": rate}
            )
            if is_moli_browser() and (not error or "CPU throttling is not supported" not in str(error)):
                raise SmokeError(f"CPU rate {rate} should report unsupported: {error}")
            if not is_moli_browser():
                assert_equal(error, None, "Chromium accepts CPU throttling")
            for neutral_rate in [1.0, 0.5, 0.0, -1.0]:
                result = await cdp.send("Emulation.setCPUThrottlingRate", {"rate": neutral_rate})
                assert_equal(result, {}, f"CPU rate {neutral_rate} disables throttling")
            result = await cdp.send("Runtime.evaluate", {"expression": "1 + 1", "returnByValue": True})
            assert_equal(result.get("result", {}).get("value"), 2, "page remains usable after rejected throttling")
    finally:
        await second_page.close()
    state.record("cpu_throttling_capability_multiple_pages_sample")

async def _verify_chromium_profiler_cpu_profile_sample(state: SmokeState) -> None:
    await _navigate_with_cdp_until_dom_ready(state, f"{state.fixture}/plain")
    await state.cdp.send("Profiler.enable")
    await state.cdp.send("Profiler.setSamplingInterval", {"interval": 100})
    await state.cdp.send("Profiler.start")
    try:
        burn = await state.cdp.send(
            "Runtime.evaluate",
            {
                "expression": """
                    (() => {
                        function chromiumProfilerSmokeWork() {
                            let total = 0;
                            for (let i = 0; i < 50000; ++i)
                                total += Math.sqrt(i);
                            return total > 0;
                        }
                        return chromiumProfilerSmokeWork();
                    })()
                """,
                "returnByValue": True,
            },
        )
        assert_equal(
            burn.get("result", {}).get("value"),
            True,
            "Chromium Profiler CPU profile work sample",
        )
        profile_result = await state.cdp.send("Profiler.stop")
    finally:
        await state.cdp.send("Profiler.disable")

    profile = profile_result.get("profile") or {}
    if not isinstance(profile.get("startTime"), (int, float)) or not isinstance(profile.get("endTime"), (int, float)):
        raise SmokeError(f"Profiler.stop should return startTime/endTime: {profile_result}")
    nodes = profile.get("nodes") or []
    if not isinstance(nodes, list) or not nodes:
        raise SmokeError(f"Profiler.stop should return non-empty profile nodes: {profile_result}")
    _assert_profile_tree_shape(profile, "Profiler.stop CPU profile")
    if profile["endTime"] < profile["startTime"]:
        raise SmokeError(f"Profiler profile time range should be monotonic: {profile_result}")
    state.record("chromium_profiler_cpu_profile_sample")

async def _verify_profiler_after_rejected_cpu_throttling_sample(state: SmokeState) -> None:
    await _navigate_with_cdp_until_dom_ready(state, f"{state.fixture}/plain?profiler-cpu-throttling")
    await state.cdp.send("Profiler.enable")
    await state.cdp.send("Profiler.setSamplingInterval", {"interval": 100})
    await state.cdp.send("Profiler.start")
    try:
        error = await _send_cdp_expect_optional_error(
            state.cdp, "Emulation.setCPUThrottlingRate", {"rate": 4.0}
        )
        if is_moli_browser() and (not error or "CPU throttling is not supported" not in str(error)):
            raise SmokeError(f"CPU throttling while profiling should report unsupported: {error}")
        if not is_moli_browser():
            assert_equal(error, None, "Chromium accepts CPU throttling while profiling")
        burn = await state.cdp.send(
            "Runtime.evaluate",
            {
                "expression": """
                    (() => {
                        function profilerWorkAfterRejectedThrottle() {
                            let count = 0;
                            const limit = 10000000;
                            const target = Date.now() + 1000;
                            for (let i = 0; i < limit && Date.now() < target; ++i)
                                count += i;
                            return count >= 0;
                        }
                        return profilerWorkAfterRejectedThrottle();
                    })()
                """,
                "returnByValue": True,
            },
        )
        assert_equal(
            burn.get("result", {}).get("value"),
            True,
            "Profiler work after rejected CPU throttling",
        )
        profile_result = await state.cdp.send("Profiler.stop")
    finally:
        await state.cdp.send("Profiler.disable")
        await state.cdp.send("Emulation.setCPUThrottlingRate", {"rate": 1})

    profile = profile_result.get("profile") or {}
    nodes = profile.get("nodes") or []
    if not isinstance(nodes, list) or not nodes:
        raise SmokeError(f"Profiler.stop should return non-empty profile nodes after rejected throttling: {profile_result}")
    _assert_profile_tree_shape(profile, "Profiler after rejected CPU throttling")
    if "profilerWorkAfterRejectedThrottle" not in _profile_function_names(profile):
        raise SmokeError(f"CPU profile after rejected throttling should include sampled work frame: {profile_result}")
    state.record("profiler_after_rejected_cpu_throttling_sample")

async def _verify_chromium_profiler_stop_without_start_sample(state: SmokeState) -> None:
    await _navigate_with_cdp_until_dom_ready(state, f"{state.fixture}/plain?profiler-stop-without-start")
    stop_error = await _send_cdp_expect_optional_error(state.cdp, "Profiler.stop", {})
    if not stop_error or "No recording profiles found" not in str(stop_error):
        raise SmokeError(f"Profiler.stop without Profiler.start should return recording-not-found error: {stop_error}")
    state.record("chromium_profiler_stop_without_start_sample")

async def _verify_chromium_profiler_sampling_interval_contract_sample(state: SmokeState) -> None:
    await _navigate_with_cdp_until_dom_ready(state, f"{state.fixture}/plain?profiler-sampling-interval")
    await state.cdp.send("Profiler.setSamplingInterval", {"interval": 100})
    await state.cdp.send("Profiler.enable")
    await state.cdp.send("Profiler.start")
    try:
        interval_error = await _send_cdp_expect_optional_error(
            state.cdp,
            "Profiler.setSamplingInterval",
            {"interval": 200},
        )
        if not interval_error or "Cannot change sampling interval" not in str(interval_error):
            raise SmokeError(
                "Profiler.setSamplingInterval while recording should return Chromium error: "
                f"{interval_error}"
            )
        stopped = await state.cdp.send("Profiler.stop")
        if not (stopped.get("profile") or {}).get("nodes"):
            raise SmokeError(f"Profiler.stop after sampling interval contract sample should return a profile: {stopped}")
    finally:
        await state.cdp.send("Profiler.disable")
    state.record("chromium_profiler_sampling_interval_contract_sample")

async def _verify_chromium_profiler_enable_disable_contract_sample(state: SmokeState) -> None:
    await _navigate_with_cdp_until_dom_ready(state, f"{state.fixture}/plain?profiler-enable-disable")
    events = attach_cdp_event_collector(
        state.cdp,
        ["Profiler.consoleProfileStarted", "Profiler.consoleProfileFinished"],
    )
    start = len(events)

    start_error = await _send_cdp_expect_optional_error(state.cdp, "Profiler.start", {})
    if not start_error or "Profiler is not enabled" not in str(start_error):
        raise SmokeError(f"Profiler.start without Profiler.enable should return Chromium error: {start_error}")

    await state.cdp.send("Profiler.enable")
    await state.cdp.send("Profiler.start")
    await state.cdp.send(
        "Runtime.evaluate",
        {
            "expression": "console.profile('chromium-cdp-enable-disable-console-profile')",
            "returnByValue": True,
        },
    )
    await wait_until(
        lambda: _has_event(events[start:], "Profiler.consoleProfileStarted"),
        "Chromium Profiler enable-disable console profile start event",
    )

    await state.cdp.send("Profiler.disable")
    await state.cdp.send("Profiler.enable")
    stop_error = await _send_cdp_expect_optional_error(state.cdp, "Profiler.stop", {})
    if not stop_error or "No recording profiles found" not in str(stop_error):
        raise SmokeError(f"Profiler.disable should stop frontend initiated profile: {stop_error}")

    profile_end = await state.cdp.send(
        "Runtime.evaluate",
        {
            "expression": "console.profileEnd('chromium-cdp-enable-disable-console-profile')",
            "returnByValue": True,
        },
    )
    if profile_end.get("exceptionDetails"):
        raise SmokeError(f"console.profileEnd after Profiler.disable should not throw: {profile_end}")
    await state.cdp.send("Profiler.disable")
    state.record("chromium_profiler_enable_disable_contract_sample")

async def _verify_chromium_profiler_console_profile_sample(state: SmokeState) -> None:
    await _navigate_with_cdp_until_dom_ready(state, f"{state.fixture}/plain?profiler-console-profile")
    events = attach_cdp_event_collector(
        state.cdp,
        ["Profiler.consoleProfileStarted", "Profiler.consoleProfileFinished"],
    )
    await state.cdp.send("Profiler.enable")
    await state.cdp.send("Profiler.setSamplingInterval", {"interval": 100})
    start = len(events)
    try:
        evaluated = await state.cdp.send(
            "Runtime.evaluate",
            {
                "expression": """
                    (() => {
                        function chromiumConsoleProfileSmokeWork() {
                            let total = 0;
                            for (let i = 0; i < 500000; ++i)
                                total += Math.sqrt(i + 1);
                            return total > 0;
                        }
                        console.profile('chromium-cdp-console-profile');
                        const result = chromiumConsoleProfileSmokeWork();
                        console.profileEnd('chromium-cdp-console-profile');
                        return result;
                    })()
                """,
                "returnByValue": True,
            },
        )
        assert_equal(
            evaluated.get("result", {}).get("value"),
            True,
            "Chromium Profiler console profile work sample",
        )
        await wait_until(
            lambda: _has_event(events[start:], "Profiler.consoleProfileStarted")
            and _has_event(events[start:], "Profiler.consoleProfileFinished"),
            "Chromium Profiler console profile started/finished events",
        )

        try:
            await state.cdp.send("Profiler.stop")
        except Exception as error:
            if "No recording profiles found" not in str(error):
                raise SmokeError(f"Profiler.stop after console.profile should report no frontend recording: {error}") from error
        else:
            raise SmokeError("console.profile must not create a frontend Profiler.start recording")
    finally:
        await state.cdp.send("Profiler.disable")

    started = next(event for event in events[start:] if event["method"] == "Profiler.consoleProfileStarted")
    finished = next(event for event in events[start:] if event["method"] == "Profiler.consoleProfileFinished")
    if started["params"].get("title") != "chromium-cdp-console-profile":
        raise SmokeError(f"consoleProfileStarted should include requested title: {started}")
    if finished["params"].get("title") != "chromium-cdp-console-profile":
        raise SmokeError(f"consoleProfileFinished should include requested title: {finished}")
    if not started["params"].get("id") or finished["params"].get("id") != started["params"].get("id"):
        raise SmokeError(f"console profile start/finish ids should match: started={started}, finished={finished}")
    profile = finished["params"].get("profile") or {}
    _assert_profile_tree_shape(profile, "consoleProfileFinished profile")
    if "chromiumConsoleProfileSmokeWork" not in _profile_function_names(profile):
        raise SmokeError(f"consoleProfileFinished profile should include sampled page work: {finished}")
    state.record("chromium_profiler_console_profile_sample")

async def _verify_chromium_profiler_nested_console_profile_sample(state: SmokeState) -> None:
    await _navigate_with_cdp_until_dom_ready(state, f"{state.fixture}/plain?profiler-nested-console-profile")
    events = attach_cdp_event_collector(
        state.cdp,
        ["Profiler.consoleProfileFinished"],
    )
    await state.cdp.send("Profiler.enable")
    await state.cdp.send("Profiler.setSamplingInterval", {"interval": 100})
    start = len(events)
    try:
        evaluated = await state.cdp.send(
            "Runtime.evaluate",
            {
                "expression": """
                    (() => {
                        function collectProfiles() {
                            function chromiumNestedConsoleProfileBurn(seed) {
                                let total = seed;
                                for (let i = 0; i < 500000; ++i)
                                    total += Math.sqrt(i + seed);
                                return total > 0;
                            }
                            console.profile('outer');
                            chromiumNestedConsoleProfileBurn(1);
                            console.profile(42);
                            chromiumNestedConsoleProfileBurn(2);
                            console.profileEnd('outer');
                            chromiumNestedConsoleProfileBurn(3);
                            console.profileEnd(42);
                            return true;
                        }
                        return collectProfiles();
                    })()
                """,
                "returnByValue": True,
            },
        )
        assert_equal(
            evaluated.get("result", {}).get("value"),
            True,
            "Chromium Profiler nested console profile work sample",
        )
        await wait_until(
            lambda: len(_events_with_method(events[start:], "Profiler.consoleProfileFinished")) >= 2,
            "Chromium nested console profile finished events",
        )
    finally:
        await state.cdp.send("Profiler.disable")

    finished = _events_with_method(events[start:], "Profiler.consoleProfileFinished")
    if len(finished) != 2:
        raise SmokeError(f"Chromium console-profile.js should finish exactly two profiles: {finished}")
    if not any(event["params"].get("title") == "outer" for event in finished):
        raise SmokeError(f"Nested console profile should finish the outer profile: {finished}")
    numeric_profile = next((event for event in finished if event["params"].get("title") == "42"), None)
    if not numeric_profile:
        raise SmokeError(f"Nested console profile should stringify numeric title 42: {finished}")
    profile = numeric_profile["params"].get("profile") or {}
    _assert_profile_tree_shape(profile, "nested consoleProfileFinished profile")
    if "collectProfiles" not in _profile_function_names(profile):
        raise SmokeError(f"Numeric nested profile should include collectProfiles frame: {numeric_profile}")
    state.record("chromium_profiler_nested_console_profile_sample")

async def _verify_chromium_profiler_parameterless_profile_end_sample(state: SmokeState) -> None:
    await _navigate_with_cdp_until_dom_ready(state, f"{state.fixture}/plain?profiler-parameterless-profile-end")
    events = attach_cdp_event_collector(
        state.cdp,
        ["Profiler.consoleProfileFinished"],
    )
    await state.cdp.send("Profiler.enable")
    start = len(events)
    try:
        evaluated = await state.cdp.send(
            "Runtime.evaluate",
            {
                "expression": """
                    (() => {
                        function collectProfiles() {
                            console.profile();
                            console.profile('titled');
                            console.profileEnd('titled');
                            console.profileEnd();
                            return true;
                        }
                        return collectProfiles();
                    })()
                """,
                "returnByValue": True,
            },
        )
        assert_equal(
            evaluated.get("result", {}).get("value"),
            True,
            "Chromium Profiler parameterless profileEnd work sample",
        )
        await wait_until(
            lambda: len(_events_with_method(events[start:], "Profiler.consoleProfileFinished")) >= 2,
            "Chromium parameterless profileEnd finished events",
        )

        try:
            await state.cdp.send("Profiler.stop")
        except Exception as error:
            if "No recording profiles found" not in str(error):
                raise SmokeError(f"Profiler.stop after parameterless profileEnd should report no frontend recording: {error}") from error
        else:
            raise SmokeError("parameterless console.profileEnd must not create a frontend Profiler.start recording")
    finally:
        await state.cdp.send("Profiler.disable")

    finished = _events_with_method(events[start:], "Profiler.consoleProfileFinished")
    if len(finished) != 2:
        raise SmokeError(f"Chromium console-profileEnd-parameterless-crash.js should finish exactly two profiles: {finished}")
    if not any(event["params"].get("title") == "titled" for event in finished):
        raise SmokeError(f"Parameterless profileEnd sample should finish titled profile: {finished}")
    state.record("chromium_profiler_parameterless_profile_end_sample")

async def _verify_chromium_profiler_navigation_profile_continuity_sample(state: SmokeState) -> None:
    await _navigate_with_cdp_until_dom_ready(state, f"{state.fixture}/plain?profiler-navigation-before")
    await state.cdp.send("Profiler.enable")
    await state.cdp.send("Profiler.setSamplingInterval", {"interval": 100})
    await state.cdp.send("Profiler.start")
    try:
        before = await state.cdp.send(
            "Runtime.evaluate",
            {
                "expression": """
                    (() => {
                        function chromiumProfilerBeforeNavigationWork() {
                            let total = 0;
                            for (let i = 0; i < 250000; ++i)
                                total += Math.sqrt(i);
                            return total > 0;
                        }
                        return chromiumProfilerBeforeNavigationWork();
                    })()
                """,
                "returnByValue": True,
            },
        )
        assert_equal(
            before.get("result", {}).get("value"),
            True,
            "Chromium Profiler pre-navigation work sample",
        )

        await state.cdp.send(
            "Page.navigate",
            {"url": f"{state.fixture}/plain?profiler-navigation-after"},
        )
        await state.page.wait_for_load_state("load", timeout=10_000)

        after = await state.cdp.send(
            "Runtime.evaluate",
            {
                "expression": """
                    (() => {
                        function chromiumProfilerAfterNavigationWork() {
                            let total = 0;
                            for (let i = 0; i < 250000; ++i)
                                total += Math.sqrt(i + 1);
                            return total > 0;
                        }
                        return chromiumProfilerAfterNavigationWork();
                    })()
                """,
                "returnByValue": True,
            },
        )
        assert_equal(
            after.get("result", {}).get("value"),
            True,
            "Chromium Profiler post-navigation work sample",
        )
        profile_result = await state.cdp.send("Profiler.stop")
    finally:
        await state.cdp.send("Profiler.disable")

    profile = profile_result.get("profile") or {}
    _assert_profile_tree_shape(profile, "Profiler.stop navigation continuity profile")
    function_names = _profile_function_names(profile)
    # A document replacement creates a new isolate-local Profiler backend. The recording control
    # state is restored, but old-isolate samples are intentionally not merged into Profiler.stop.
    # The pre-navigation evaluation above proves that recording began before navigation; this
    # external smoke only requires the replacement backend to keep recording without a second
    # Profiler.start and return its own samples.
    if "chromiumProfilerAfterNavigationWork" not in function_names:
        raise SmokeError(
            "Profiler.stop after navigation should include post-navigation work "
            f"function; names={sorted(function_names)}"
        )
    state.record("chromium_profiler_navigation_profile_continuity_sample")

async def _verify_chromium_profiler_attached_session_navigation_profile_continuity_sample(state: SmokeState) -> None:
    await _navigate_with_cdp_until_dom_ready(state, f"{state.fixture}/plain?profiler-attached-navigation-before")
    attached_cdp = await new_cdp_session(state.context, state.page)
    await state.cdp.send("Profiler.enable")
    await attached_cdp.send("Profiler.enable")
    await attached_cdp.send("Profiler.setSamplingInterval", {"interval": 100})
    await attached_cdp.send("Profiler.start")
    try:
        primary_stop_before = await _send_cdp_expect_optional_error(state.cdp, "Profiler.stop", {})
        if not primary_stop_before or "No recording profiles found" not in str(primary_stop_before):
            raise SmokeError(
                "Primary CDP session must not observe attached Profiler.start recording: "
                f"{primary_stop_before}"
            )

        before = await attached_cdp.send(
            "Runtime.evaluate",
            {
                "expression": """
                    (() => {
                        function chromiumProfilerAuxBeforeNavigationWork() {
                            let total = 0;
                            for (let i = 0; i < 250000; ++i)
                                total += Math.sqrt(i);
                            return total > 0;
                        }
                        return chromiumProfilerAuxBeforeNavigationWork();
                    })()
                """,
                "returnByValue": True,
            },
        )
        assert_equal(
            before.get("result", {}).get("value"),
            True,
            "Chromium Profiler attached pre-navigation work sample",
        )

        await attached_cdp.send(
            "Page.navigate",
            {"url": f"{state.fixture}/plain?profiler-attached-navigation-after"},
        )
        await state.page.wait_for_load_state("load", timeout=10_000)

        after = await attached_cdp.send(
            "Runtime.evaluate",
            {
                "expression": """
                    (() => {
                        function chromiumProfilerAuxAfterNavigationWork() {
                            let total = 0;
                            for (let i = 0; i < 250000; ++i)
                                total += Math.sqrt(i + 1);
                            return total > 0;
                        }
                        return chromiumProfilerAuxAfterNavigationWork();
                    })()
                """,
                "returnByValue": True,
            },
        )
        assert_equal(
            after.get("result", {}).get("value"),
            True,
            "Chromium Profiler attached post-navigation work sample",
        )

        profile_result = await attached_cdp.send("Profiler.stop")
        primary_stop_after = await _send_cdp_expect_optional_error(state.cdp, "Profiler.stop", {})
        if not primary_stop_after or "No recording profiles found" not in str(primary_stop_after):
            raise SmokeError(
                "Primary CDP session must stay isolated after attached Profiler.stop: "
                f"{primary_stop_after}"
            )
    finally:
        await attached_cdp.send("Profiler.disable")
        await state.cdp.send("Profiler.disable")

    profile = profile_result.get("profile") or {}
    _assert_profile_tree_shape(profile, "attached Profiler.stop navigation continuity profile")
    function_names = _profile_function_names(profile)
    # As above, navigation restores the attached session's recording state, not samples from the
    # disposed isolate. Session isolation and replacement-backend sampling are the stable contract.
    if "chromiumProfilerAuxAfterNavigationWork" not in function_names:
        raise SmokeError(
            "Attached Profiler.stop after navigation should include post-navigation work "
            f"function; names={sorted(function_names)}"
        )
    state.record("chromium_profiler_attached_session_navigation_profile_continuity_sample")

async def _verify_chromium_profiler_attached_session_detach_clears_state_sample(state: SmokeState) -> None:
    await _navigate_with_cdp_until_dom_ready(state, f"{state.fixture}/plain?profiler-session-detach")
    old_cdp = await new_cdp_session(state.context, state.page)
    await old_cdp.send("Profiler.enable")
    await old_cdp.send("Profiler.start")
    await old_cdp.detach()

    new_cdp = await new_cdp_session(state.context, state.page)
    try:
        start_error = await _send_cdp_expect_optional_error(new_cdp, "Profiler.start", {})
        if not start_error or "Profiler is not enabled" not in str(start_error):
            raise SmokeError(
                "New attached CDP session must not inherit detached Profiler.enable/start state: "
                f"{start_error}"
            )
        stop_error = await _send_cdp_expect_optional_error(new_cdp, "Profiler.stop", {})
        if not stop_error or "No recording profiles found" not in str(stop_error):
            raise SmokeError(
                "New attached CDP session must not inherit detached recording state: "
                f"{stop_error}"
            )
    finally:
        await new_cdp.detach()

    state.record("chromium_profiler_attached_session_detach_clears_state_sample")

async def _verify_chromium_profiler_precise_coverage_error_sample(state: SmokeState) -> None:
    await _navigate_with_cdp_until_dom_ready(state, f"{state.fixture}/plain?profiler-precise-coverage-error")
    await state.cdp.send("Profiler.enable")
    try:
        take_error = await _send_cdp_expect_optional_error(state.cdp, "Profiler.takePreciseCoverage", {})
        if not take_error or "Precise coverage has not been started" not in str(take_error):
            raise SmokeError(
                "Profiler.takePreciseCoverage before startPreciseCoverage should return Chromium error: "
                f"{take_error}"
            )
    finally:
        await state.cdp.send("Profiler.disable")

    state.record("chromium_profiler_precise_coverage_error_sample")

async def _verify_chromium_profiler_precise_coverage_sample(state: SmokeState) -> None:
    await _navigate_with_cdp_until_dom_ready(state, f"{state.fixture}/plain")
    await state.cdp.send("Profiler.enable")
    try:
        started = await state.cdp.send(
            "Profiler.startPreciseCoverage",
            {"callCount": True, "detailed": True, "allowTriggeredUpdates": False},
        )
        if not isinstance(started.get("timestamp"), (int, float)):
            raise SmokeError(f"Profiler.startPreciseCoverage should return timestamp: {started}")

        evaluated = await state.cdp.send(
            "Runtime.evaluate",
            {
                "expression": """
                    function chromiumProfilerCoverageSmoke(value) {
                        if (value > 0)
                            return value + 1;
                        return value - 1;
                    }
                    chromiumProfilerCoverageSmoke(41)
                """,
                "returnByValue": True,
            },
        )
        assert_equal(
            evaluated.get("result", {}).get("value"),
            42,
            "Chromium Profiler precise coverage work sample",
        )

        precise = await state.cdp.send("Profiler.takePreciseCoverage")
        if not isinstance(precise.get("timestamp"), (int, float)):
            raise SmokeError(f"Profiler.takePreciseCoverage should return timestamp: {precise}")
        _assert_script_coverage_array(precise, "Profiler.takePreciseCoverage")

        best_effort = await state.cdp.send("Profiler.getBestEffortCoverage")
        _assert_script_coverage_array(best_effort, "Profiler.getBestEffortCoverage")
    finally:
        await state.cdp.send("Profiler.stopPreciseCoverage")
        await state.cdp.send("Profiler.disable")

    state.record("chromium_profiler_precise_coverage_sample")

async def _verify_chromium_profiler_precise_coverage_counter_reset_sample(state: SmokeState) -> None:
    await _navigate_with_cdp_until_dom_ready(state, f"{state.fixture}/plain?profiler-coverage-counter-reset")
    await state.cdp.send("Profiler.enable")
    precise_started = False
    try:
        await state.cdp.send(
            "Profiler.startPreciseCoverage",
            {"callCount": True, "detailed": False, "allowTriggeredUpdates": False},
        )
        precise_started = True

        evaluated = await state.cdp.send(
            "Runtime.evaluate",
            {
                "expression": """
                    function chromiumProfilerCoverageCounterResetSmoke() {
                        return 41;
                    }
                    chromiumProfilerCoverageCounterResetSmoke();
                    //# sourceURL=chromium-profiler-coverage-counter-reset.js
                """,
                "returnByValue": True,
            },
        )
        assert_equal(
            evaluated.get("result", {}).get("value"),
            41,
            "Chromium Profiler coverage counter reset work sample",
        )

        first = await state.cdp.send("Profiler.takePreciseCoverage")
        first_function = _find_coverage_function(
            _find_script_coverage_by_url(first, "chromium-profiler-coverage-counter-reset.js") or {},
            "chromiumProfilerCoverageCounterResetSmoke",
        )
        if not first_function:
            raise SmokeError(f"First takePreciseCoverage should include target function: {first}")
        if _coverage_function_total_count(first_function) <= 0:
            raise SmokeError(f"First takePreciseCoverage should include executed counts: {first_function}")

        second = await state.cdp.send("Profiler.takePreciseCoverage")
        second_function = _find_coverage_function(
            _find_script_coverage_by_url(second, "chromium-profiler-coverage-counter-reset.js") or {},
            "chromiumProfilerCoverageCounterResetSmoke",
        )
        second_count = _coverage_function_total_count(second_function or {})
        if second_count != 0:
            raise SmokeError(
                "takePreciseCoverage should not report stale execution counts until code runs again: "
                f"{second}"
            )

        rerun = await state.cdp.send(
            "Runtime.evaluate",
            {
                "expression": "chromiumProfilerCoverageCounterResetSmoke()",
                "returnByValue": True,
            },
        )
        assert_equal(
            rerun.get("result", {}).get("value"),
            41,
            "Chromium Profiler coverage counter reset rerun sample",
        )

        third = await state.cdp.send("Profiler.takePreciseCoverage")
        third_function = _find_coverage_function(
            _find_script_coverage_by_url(third, "chromium-profiler-coverage-counter-reset.js") or {},
            "chromiumProfilerCoverageCounterResetSmoke",
        )
        if not third_function:
            raise SmokeError(f"Third takePreciseCoverage should include target function after rerun: {third}")
        if _coverage_function_total_count(third_function) <= 0:
            raise SmokeError(f"Coverage counters should resume after code runs again: {third_function}")
    finally:
        if precise_started:
            await state.cdp.send("Profiler.stopPreciseCoverage")
        await state.cdp.send("Profiler.disable")

    state.record("chromium_profiler_precise_coverage_counter_reset_sample")

async def _verify_chromium_profiler_precise_block_coverage_sample(state: SmokeState) -> None:
    await _navigate_with_cdp_until_dom_ready(state, f"{state.fixture}/plain?profiler-coverage-block")
    await state.cdp.send("Profiler.enable")
    precise_started = False
    try:
        await state.cdp.send(
            "Profiler.startPreciseCoverage",
            {"callCount": True, "detailed": True, "allowTriggeredUpdates": False},
        )
        precise_started = True

        evaluated = await state.cdp.send(
            "Runtime.evaluate",
            {
                "expression": """
                    function chromiumProfilerCoverageBlockSmoke(value) {
                        if (value === 0)
                            return 0;
                        if (value > 0)
                            return value + 1;
                        return value - 1;
                    }
                    chromiumProfilerCoverageBlockSmoke(41)
                    //# sourceURL=chromium-profiler-coverage-block.js
                """,
                "returnByValue": True,
            },
        )
        assert_equal(
            evaluated.get("result", {}).get("value"),
            42,
            "Chromium Profiler block coverage work sample",
        )

        coverage = await state.cdp.send("Profiler.takePreciseCoverage")
        script = _find_script_coverage_by_url(coverage, "chromium-profiler-coverage-block.js")
        if not script:
            raise SmokeError(f"Profiler.takePreciseCoverage should include sourceURL script: {coverage}")
        function = _find_coverage_function(script, "chromiumProfilerCoverageBlockSmoke")
        if not function:
            raise SmokeError(f"Profiler.takePreciseCoverage should include target function: {script}")
        if function.get("isBlockCoverage") is not True:
            raise SmokeError(f"Detailed precise coverage should report block coverage: {function}")
        ranges = function.get("ranges") or []
        if len(ranges) < 2:
            raise SmokeError(f"Block coverage should expose multiple ranges for branch function: {function}")
        counts = [range_.get("count") for range_ in ranges if isinstance(range_, dict)]
        if not any(count == 0 for count in counts) or not any(isinstance(count, int) and count > 0 for count in counts):
            raise SmokeError(f"Block coverage should expose executed and unexecuted ranges: {function}")
    finally:
        if precise_started:
            await state.cdp.send("Profiler.stopPreciseCoverage")
        await state.cdp.send("Profiler.disable")

    state.record("chromium_profiler_precise_block_coverage_sample")

async def _verify_chromium_profiler_best_effort_with_precise_coverage_sample(state: SmokeState) -> None:
    await _navigate_with_cdp_until_dom_ready(state, f"{state.fixture}/plain?profiler-best-effort-with-precise")

    async def run_case(case_name: str, start_params: dict[str, Any]) -> None:
        await state.cdp.send("Profiler.enable")
        precise_started = False
        try:
            await state.cdp.send("Profiler.startPreciseCoverage", start_params)
            precise_started = True

            function_name = f"chromiumProfilerBestEffortWithPrecise{case_name}"
            source_url = f"chromium-profiler-best-effort-with-precise-{case_name}.js"
            evaluated = await state.cdp.send(
                "Runtime.evaluate",
                {
                    "expression": f"""
                        function {function_name}(value) {{
                            if (value > 0)
                                return value + 1;
                            return value - 1;
                        }}
                        {function_name}(41)
                        //# sourceURL={source_url}
                    """,
                    "returnByValue": True,
                },
            )
            assert_equal(
                evaluated.get("result", {}).get("value"),
                42,
                f"Chromium Profiler best-effort with precise coverage work sample {case_name}",
            )

            first = await state.cdp.send("Profiler.getBestEffortCoverage")
            _assert_script_coverage_array(first, "Profiler.getBestEffortCoverage")
            _assert_coverage_contains_function(
                first,
                source_url,
                function_name,
                f"Profiler.getBestEffortCoverage with active precise coverage {case_name}",
            )

            second = await state.cdp.send("Profiler.getBestEffortCoverage")
            _assert_script_coverage_array(second, "Profiler.getBestEffortCoverage repeat")
            _assert_coverage_contains_function(
                second,
                source_url,
                function_name,
                f"repeated Profiler.getBestEffortCoverage with active precise coverage {case_name}",
            )
        finally:
            if precise_started:
                await state.cdp.send("Profiler.stopPreciseCoverage")
            await state.cdp.send("Profiler.disable")

    await run_case("Binary", {"detailed": True})
    await run_case("Count", {"callCount": True, "detailed": True})
    state.record("chromium_profiler_best_effort_with_precise_coverage_sample")


async def run_performance_group(state: SmokeState) -> None:
    await _verify_chromium_performance_enable_sample(state)
    await _verify_chromium_performance_metrics_sample(state)
    await _verify_cpu_throttling_capability_multiple_pages_sample(state)
    await _verify_chromium_profiler_cpu_profile_sample(state)
    await _verify_profiler_after_rejected_cpu_throttling_sample(state)
    await _verify_chromium_profiler_stop_without_start_sample(state)
    await _verify_chromium_profiler_sampling_interval_contract_sample(state)
    await _verify_chromium_profiler_enable_disable_contract_sample(state)
    await _verify_chromium_profiler_console_profile_sample(state)
    await _verify_chromium_profiler_nested_console_profile_sample(state)
    await _verify_chromium_profiler_parameterless_profile_end_sample(state)
    await _verify_chromium_profiler_navigation_profile_continuity_sample(state)
    await _verify_chromium_profiler_attached_session_navigation_profile_continuity_sample(state)
    await _verify_chromium_profiler_attached_session_detach_clears_state_sample(state)
    await _verify_chromium_profiler_precise_coverage_error_sample(state)
    await _verify_chromium_profiler_precise_coverage_sample(state)
    await _verify_chromium_profiler_precise_coverage_counter_reset_sample(state)
    await _verify_chromium_profiler_precise_block_coverage_sample(state)
    await _verify_chromium_profiler_best_effort_with_precise_coverage_sample(state)
