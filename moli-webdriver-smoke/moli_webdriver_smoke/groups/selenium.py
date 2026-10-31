from __future__ import annotations

import asyncio
import json
import time
import warnings
from collections.abc import Callable
from pathlib import Path
from tempfile import TemporaryDirectory
from typing import Any

from selenium import webdriver
from selenium.common.exceptions import (
    JavascriptException,
    NoSuchElementException,
    NoSuchFrameException,
    NoSuchShadowRootException,
    StaleElementReferenceException,
    TimeoutException,
    UnexpectedTagNameException,
    WebDriverException,
)
from selenium.webdriver.common.bidi.browsing_context import ReadinessState
from selenium.webdriver.common.bidi.log import LogLevel
from selenium.webdriver.common.bidi.script import RealmType, ResultOwnership
from selenium.webdriver.common.action_chains import ActionChains
from selenium.webdriver.common.by import By
from selenium.webdriver.common.keys import Keys
from selenium.webdriver.common.print_page_options import PrintOptions
from selenium.webdriver.common.window import WindowTypes
from selenium.webdriver.remote.shadowroot import ShadowRoot
from selenium.webdriver.remote.webelement import WebElement
from selenium.webdriver.support import expected_conditions as EC
from selenium.webdriver.support.relative_locator import locate_with
from selenium.webdriver.support.select import Select
from selenium.webdriver.support.ui import WebDriverWait

from ..assertions import assert_equal, assert_true, record
from ..config import WebDriverTarget
from ..scenarios import record_failure, record_progress
from ..selenium_options import create_selenium_options


SeleniumScenario = Callable[[webdriver.Remote, str, list[dict[str, Any]]], None]


async def run_selenium_group(
    target: WebDriverTarget,
    fixture: str,
    results: list[dict[str, Any]],
    continue_on_failure: bool = False,
) -> None:
    await asyncio.to_thread(
        _run_selenium_group_sync,
        target,
        fixture,
        results,
        continue_on_failure,
    )


def _run_selenium_group_sync(
    target: WebDriverTarget,
    fixture: str,
    results: list[dict[str, Any]],
    continue_on_failure: bool = False,
) -> None:
    if continue_on_failure:
        for name, scenario in _selenium_scenarios():
            record_progress("selenium", name, "start")
            try:
                _run_selenium_isolated_scenario_sync(target, fixture, results, scenario)
            except Exception as error:
                record_progress("selenium", name, "fail")
                record_failure(results, "selenium", name, error)
            else:
                record_progress("selenium", name, "done")
        return

    options = create_selenium_options(target, enable_downloads=True)
    options.enable_bidi = True
    driver = webdriver.Remote(command_executor=target.endpoint, options=options)
    record(results, "selenium_session_new", {"sessionId": driver.session_id})
    try:
        for name, scenario in _selenium_scenarios():
            record_progress("selenium", name, "start")
            scenario(driver, fixture, results)
            record_progress("selenium", name, "done")
    finally:
        driver.quit()
        record(results, "selenium_session_delete")


def _selenium_scenarios() -> tuple[tuple[str, SeleniumScenario], ...]:
    return (
        ("selenium_bidi_facade", _run_selenium_bidi_facade_smoke),
        ("selenium_bidi_browsing_context_events", _run_selenium_bidi_browsing_context_event_smoke),
        ("selenium_bidi_network_handler", _run_selenium_bidi_network_handler_smoke),
        ("selenium_bidi_network_auth_handler", _run_selenium_bidi_network_auth_handler_smoke),
        ("selenium_bidi_browser_emulation", _run_selenium_bidi_browser_emulation_smoke),
        ("selenium_bidi_preload_scripts", _run_selenium_bidi_preload_script_smoke),
        ("selenium_bidi_script_handles", _run_selenium_bidi_script_handle_smoke),
        ("selenium_bidi_script_serialization", _run_selenium_bidi_script_serialization_smoke),
        ("selenium_navigation_element_script", _run_selenium_navigation_element_script_smoke),
        ("selenium_script_return_and_pinning", _run_selenium_script_return_and_pinning_smoke),
        ("selenium_async_script", _run_selenium_async_script_smoke),
        ("selenium_text_handling", _run_selenium_text_handling_smoke),
        ("selenium_locator_accessibility", _run_selenium_locator_accessibility_smoke),
        ("selenium_relative_locator", _run_selenium_relative_locator_smoke),
        ("selenium_network_storage", _run_selenium_network_storage_smoke),
        ("selenium_remote_downloads", _run_selenium_remote_downloads_smoke),
        ("selenium_cookie_management", _run_selenium_cookie_management_smoke),
        ("selenium_wait_expected_conditions", _run_selenium_wait_expected_conditions_smoke),
        ("selenium_implicit_wait", _run_selenium_implicit_wait_smoke),
        ("selenium_stale_element_references", _run_selenium_stale_element_smoke),
        ("selenium_document_open_replacement_stale_element", _run_selenium_document_open_replacement_stale_element_smoke),
        ("selenium_nested_frame_switching", _run_selenium_nested_frame_switching_smoke),
        ("selenium_deleted_frame_recovery", _run_selenium_deleted_frame_recovery_smoke),
        ("selenium_form_submit_typing", _run_selenium_form_submit_typing_smoke),
        ("selenium_form_file_actions", _run_selenium_form_file_actions_scenario),
        ("selenium_pointer_actions", _run_selenium_pointer_actions_smoke),
        (
            "selenium_screenshot_print_unsupported_cookie_window",
            _run_selenium_screenshot_print_unsupported_cookie_window_smoke,
        ),
        ("selenium_popup_window_workflow", _run_selenium_popup_window_workflow_smoke),
        ("selenium_shadow_root_dialog", _run_selenium_shadow_root_dialog_scenario),
    )


def _run_selenium_isolated_scenario_sync(
    target: WebDriverTarget,
    fixture: str,
    results: list[dict[str, Any]],
    scenario: SeleniumScenario,
) -> None:
    options = create_selenium_options(target, enable_downloads=True)
    options.enable_bidi = True
    driver = webdriver.Remote(command_executor=target.endpoint, options=options)
    record(results, "selenium_session_new", {"sessionId": driver.session_id})
    scenario_error: BaseException | None = None
    try:
        scenario(driver, fixture, results)
    except Exception as error:
        scenario_error = error
    finally:
        try:
            driver.quit()
            record(results, "selenium_session_delete")
        except Exception as error:
            if scenario_error is None:
                scenario_error = error
    if scenario_error is not None:
        raise scenario_error


def _run_selenium_form_file_actions_scenario(
    driver: webdriver.Remote,
    fixture: str,
    results: list[dict[str, Any]],
) -> None:
    driver.get(f"{fixture}/webdriver/basic")
    _run_selenium_form_file_actions_smoke(driver, results)


def _run_selenium_shadow_root_dialog_scenario(
    driver: webdriver.Remote,
    fixture: str,
    results: list[dict[str, Any]],
) -> None:
    driver.get(f"{fixture}/webdriver/basic")
    _run_selenium_shadow_root_and_dialog_smoke(driver, results)


def _run_selenium_bidi_facade_smoke(
    driver: webdriver.Remote,
    fixture: str,
    results: list[dict[str, Any]],
) -> None:
    # Reduced from Selenium's bidi_browsing_context_tests.py and bidi_tests.py.
    web_socket_url = driver.capabilities.get("webSocketUrl")
    assert_true(
        isinstance(web_socket_url, str) and web_socket_url.endswith(f"/session/{driver.session_id}"),
        "Selenium BiDi facade webSocketUrl should be session scoped",
    )

    context = driver.browsing_context.create(type=WindowTypes.TAB)
    assert_true(isinstance(context, str) and context, "Selenium BiDi facade creates a context")
    try:
        page_url = f"{fixture}/webdriver/basic"
        navigation = driver.browsing_context.navigate(
            context=context,
            url=page_url,
            wait=ReadinessState.COMPLETE,
        )
        assert_equal(navigation["url"], page_url, "Selenium BiDi facade navigate URL")

        tree = driver.browsing_context.get_tree(root=context)
        assert_equal(len(tree), 1, "Selenium BiDi facade get_tree root count")
        assert_equal(tree[0].context, context, "Selenium BiDi facade get_tree context")
        assert_equal(tree[0].url, page_url, "Selenium BiDi facade get_tree URL")

        nodes = driver.browsing_context.locate_nodes(
            context=context,
            locator={"type": "css", "value": "main"},
            max_node_count=1,
        )
        assert_equal(len(nodes), 1, "Selenium BiDi facade locate_nodes count")
        assert_true(
            isinstance(nodes[0].get("sharedId"), str) and nodes[0]["sharedId"],
            "Selenium BiDi facade locate_nodes sharedId",
        )

        evaluation = driver.script.evaluate(
            "console.log('facade hello', 7); 'eval ok'",
            {"context": context},
            False,
        )
        assert_equal(evaluation["type"], "success", "Selenium BiDi facade script.evaluate type")
        assert_equal(
            evaluation["result"]["value"],
            "eval ok",
            "Selenium BiDi facade script.evaluate result",
        )

        log_entries = []
        handler_id = driver.script.add_console_message_handler(log_entries.append)
        try:
            driver.script.evaluate(
                "console.log('facade handler', 9)",
                {"context": context},
                False,
            )
            log_entry = WebDriverWait(driver, 5, poll_frequency=0.05).until(
                lambda _: next(
                    (entry for entry in log_entries if "facade handler" in entry.text),
                    None,
                )
            )
            assert_equal(log_entry.level, LogLevel.INFO, "Selenium BiDi facade console level")
            assert_equal(log_entry.method, "log", "Selenium BiDi facade console method")
            assert_equal(log_entry.type_, "console", "Selenium BiDi facade console type")
            assert_true(
                "facade handler" in log_entry.text and "9" in log_entry.text,
                "Selenium BiDi facade console text",
            )
        finally:
            driver.script.remove_console_message_handler(handler_id)

        javascript_errors = []
        error_handler_id = driver.script.add_javascript_error_handler(javascript_errors.append)
        try:
            error_result = driver.script.evaluate(
                "setTimeout(() => { throw new Error('facade javascript error'); }, 0); 'error armed'",
                {"context": context},
                False,
            )
            assert_equal(
                error_result["result"]["value"],
                "error armed",
                "Selenium BiDi facade javascript error setup result",
            )
            error_entry = WebDriverWait(driver, 5, poll_frequency=0.05).until(
                lambda _: next(
                    (entry for entry in javascript_errors if "facade javascript error" in entry.text),
                    None,
                )
            )
            assert_equal(
                error_entry.level,
                LogLevel.ERROR,
                "Selenium BiDi facade javascript error level",
            )
            assert_equal(
                error_entry.type_,
                "javascript",
                "Selenium BiDi facade javascript error type",
            )
            assert_true(
                "Error: facade javascript error" in error_entry.text,
                "Selenium BiDi facade javascript error text",
            )
            record(results, "selenium_bidi_javascript_error_handler", {"context": context})
        finally:
            driver.script.remove_javascript_error_handler(error_handler_id)
    finally:
        driver.browsing_context.close(context)

    record(results, "selenium_bidi_facade", {"context": context})


def _run_selenium_bidi_browsing_context_event_smoke(
    driver: webdriver.Remote,
    fixture: str,
    results: list[dict[str, Any]],
) -> None:
    # Reduced from Selenium's bidi_browsing_context_tests.py event handler cases.
    context = driver.browsing_context.create(type=WindowTypes.TAB)
    assert_true(isinstance(context, str) and context, "Selenium BiDi event context")
    try:
        page_url = f"{fixture}/webdriver/basic"
        driver.browsing_context.navigate(
            context=context,
            url=page_url,
            wait=ReadinessState.COMPLETE,
        )

        fragment_events = []
        fragment_handler_id = driver.browsing_context.add_event_handler(
            "fragment_navigated",
            fragment_events.append,
            contexts=[context],
        )
        try:
            expected_fragment_url = f"{page_url}#selenium-fragment"
            fragment_result = driver.script.evaluate(
                "location.hash = 'selenium-fragment'; location.href",
                {"context": context},
                False,
            )
            assert_equal(
                fragment_result["result"]["value"],
                expected_fragment_url,
                "Selenium BiDi fragment navigation script result",
            )
            fragment_event = WebDriverWait(driver, 5, poll_frequency=0.05).until(
                lambda _: fragment_events[0] if fragment_events else False
            )
            assert_equal(
                fragment_event.context,
                context,
                "Selenium BiDi fragment event context",
            )
            assert_equal(
                fragment_event.url,
                expected_fragment_url,
                "Selenium BiDi fragment event URL",
            )
            assert_equal(fragment_event.navigation, None, "Selenium BiDi fragment event navigation")
        finally:
            driver.browsing_context.remove_event_handler("fragment_navigated", fragment_handler_id)

        history_events = []
        history_handler_id = driver.browsing_context.add_event_handler(
            "history_updated",
            history_events.append,
            contexts=[context],
        )
        try:
            expected_history_url = f"{fixture}/webdriver/bidi-history-updated"
            history_result = driver.script.evaluate(
                "history.pushState({selenium: true}, '', '/webdriver/bidi-history-updated'); location.href",
                {"context": context},
                False,
            )
            assert_equal(
                history_result["result"]["value"],
                expected_history_url,
                "Selenium BiDi history update script result",
            )
            history_event = WebDriverWait(driver, 5, poll_frequency=0.05).until(
                lambda _: history_events[0] if history_events else False
            )
            assert_equal(
                history_event.context,
                context,
                "Selenium BiDi history event context",
            )
            assert_equal(
                history_event.url,
                expected_history_url,
                "Selenium BiDi history event URL",
            )
        finally:
            driver.browsing_context.remove_event_handler("history_updated", history_handler_id)
    finally:
        driver.browsing_context.close(context)

    record(results, "selenium_bidi_browsing_context_events", {"context": context})


def _run_selenium_bidi_preload_script_smoke(
    driver: webdriver.Remote,
    fixture: str,
    results: list[dict[str, Any]],
) -> None:
    # Reduced from Selenium Python bidi_script_tests.py preload lifecycle,
    # channel-argument, sandbox, and user-context cases. This also mirrors
    # Playwright page-add-init-script.spec.ts multiple script/dispose behavior.
    context = driver.browsing_context.create(type=WindowTypes.TAB)
    first_script_id = None
    second_script_id = None
    channel_script_id = None
    sandbox_script_id = None
    user_context_script_id = None
    user_context = None
    user_context_context = None
    message_handler_id = None

    def message_field(message: Any, field: str) -> Any:
        if isinstance(message, dict):
            return message.get(field)
        return getattr(message, field, None)

    def remote_value_value(value: Any) -> Any:
        if isinstance(value, dict):
            return value.get("value")
        return getattr(value, "value", None)

    try:
        messages: list[Any] = []
        message_handler_id = driver.script.add_event_handler(
            "message",
            messages.append,
            contexts=[context],
        )
        first_script_id = driver.script._add_preload_script(
            "() => { window.__seleniumPreloadFirst = 'first'; }",
            contexts=[context],
        )
        second_script_id = driver.script._add_preload_script(
            "() => { window.__seleniumPreloadSecond = 'second'; }",
            contexts=[context],
        )
        channel_script_id = driver.script._add_preload_script(
            (
                "(channelFunc) => { "
                "channelFunc('preload-channel:' + location.pathname); "
                "window.__seleniumPreloadChannel = 'received'; "
                "}"
            ),
            arguments=[
                {
                    "type": "channel",
                    "value": {
                        "channel": "selenium-preload-channel",
                        "ownership": "none",
                    },
                }
            ],
            contexts=[context],
        )
        sandbox_script_id = driver.script._add_preload_script(
            "() => { window.__seleniumSandboxPreload = 'sandboxed'; }",
            contexts=[context],
            sandbox="selenium-preload-sandbox",
        )
        assert_true(
            isinstance(first_script_id, str) and first_script_id,
            "Selenium BiDi first preload script id",
        )
        assert_true(
            isinstance(second_script_id, str) and second_script_id,
            "Selenium BiDi second preload script id",
        )
        assert_true(
            isinstance(channel_script_id, str) and channel_script_id,
            "Selenium BiDi channel preload script id",
        )
        assert_true(
            isinstance(sandbox_script_id, str) and sandbox_script_id,
            "Selenium BiDi sandbox preload script id",
        )

        driver.browsing_context.navigate(
            context=context,
            url=f"{fixture}/webdriver/basic?selenium-preload",
            wait=ReadinessState.COMPLETE,
        )
        snapshot = driver.script._evaluate(
            (
                "JSON.stringify({"
                "first: window.__seleniumPreloadFirst ?? null, "
                "second: window.__seleniumPreloadSecond ?? null, "
                "channel: window.__seleniumPreloadChannel ?? null, "
                "title: document.title"
                "})"
            ),
            {"context": context},
            await_promise=False,
        )
        assert_equal(
            snapshot.result["value"],
            '{"first":"first","second":"second","channel":"received","title":"WebDriver Smoke Basic"}',
            "Selenium BiDi preload scripts run on navigation",
        )
        channel_message = WebDriverWait(driver, 5, poll_frequency=0.05).until(
            lambda _driver: next(
                (
                    message
                    for message in messages
                    if message_field(message, "channel") == "selenium-preload-channel"
                ),
                False,
            )
        )
        assert_equal(
            remote_value_value(message_field(channel_message, "data")),
            "preload-channel:/webdriver/basic",
            "Selenium BiDi preload channel argument emits script.message",
        )
        sandbox_default = driver.script._evaluate(
            "typeof window.__seleniumSandboxPreload",
            {"context": context},
            await_promise=False,
        )
        assert_equal(
            sandbox_default.result["value"],
            "undefined",
            "Selenium BiDi sandbox preload is isolated from default realm",
        )
        sandbox_result = driver.script._evaluate(
            "window.__seleniumSandboxPreload",
            {"context": context, "sandbox": "selenium-preload-sandbox"},
            await_promise=False,
        )
        assert_equal(
            sandbox_result.result["value"],
            "sandboxed",
            "Selenium BiDi sandbox preload evaluates in sandbox realm",
        )
        sandbox_realms = driver.script.get_realms(context=context).get("realms", [])
        assert_true(
            any(
                isinstance(realm, dict)
                and realm.get("context") == context
                and realm.get("sandbox") == "selenium-preload-sandbox"
                for realm in sandbox_realms
            ),
            "Selenium BiDi getRealms reports sandbox realm",
        )

        driver.script._remove_preload_script(script_id=first_script_id)
        first_script_id = None
        driver.browsing_context.navigate(
            context=context,
            url=f"{fixture}/webdriver/basic?selenium-preload-removed",
            wait=ReadinessState.COMPLETE,
        )
        after_remove = driver.script._evaluate(
            (
                "JSON.stringify({"
                "first: window.__seleniumPreloadFirst ?? null, "
                "second: window.__seleniumPreloadSecond ?? null"
                "})"
            ),
            {"context": context},
            await_promise=False,
        )
        assert_equal(
            after_remove.result["value"],
            '{"first":null,"second":"second"}',
            "Selenium BiDi removePreloadScript prevents removed script replay",
        )

        user_context = driver.browser.create_user_context()
        user_context_script_id = driver.script._add_preload_script(
            "() => { window.__seleniumUserContextPreload = 'user-context'; }",
            user_contexts=[user_context],
        )
        assert_true(
            isinstance(user_context_script_id, str) and user_context_script_id,
            "Selenium BiDi userContext preload script id",
        )
        user_context_context = driver.browsing_context.create(
            type=WindowTypes.TAB,
            user_context=user_context,
        )
        driver.browsing_context.navigate(
            context=user_context_context,
            url=f"{fixture}/webdriver/basic?selenium-preload-user-context",
            wait=ReadinessState.COMPLETE,
        )
        user_context_result = driver.script._evaluate(
            "window.__seleniumUserContextPreload",
            {"context": user_context_context},
            await_promise=False,
        )
        assert_equal(
            user_context_result.result["value"],
            "user-context",
            "Selenium BiDi preload script applies to matching userContext",
        )
        driver.browsing_context.navigate(
            context=context,
            url=f"{fixture}/webdriver/basic?selenium-preload-default-context",
            wait=ReadinessState.COMPLETE,
        )
        default_user_context_result = driver.script._evaluate(
            "typeof window.__seleniumUserContextPreload",
            {"context": context},
            await_promise=False,
        )
        assert_equal(
            default_user_context_result.result["value"],
            "undefined",
            "Selenium BiDi preload script does not leak across userContexts",
        )
    finally:
        if message_handler_id is not None:
            try:
                driver.script.remove_event_handler("message", message_handler_id)
            except Exception:
                pass
        for script_id in (
            first_script_id,
            second_script_id,
            channel_script_id,
            sandbox_script_id,
            user_context_script_id,
        ):
            if script_id:
                try:
                    driver.script._remove_preload_script(script_id=script_id)
                except Exception:
                    pass
        if user_context_context is not None:
            try:
                driver.browsing_context.close(user_context_context)
            except Exception:
                pass
        if user_context is not None:
            try:
                driver.browser.remove_user_context(user_context)
            except Exception:
                pass
        try:
            driver.browsing_context.close(context)
        except Exception:
            pass

    record(results, "selenium_bidi_preload_scripts", {"context": context})


def _run_selenium_bidi_script_handle_smoke(
    driver: webdriver.Remote,
    fixture: str,
    results: list[dict[str, Any]],
) -> None:
    # Reduced from Selenium Python bidi_script_tests.py get_realms,
    # result_ownership, call_function, and disown_handles cases. This also
    # mirrors Playwright JSHandle argument/handle lifecycle workflows.
    context = driver.browsing_context.create(type=WindowTypes.TAB)
    try:
        driver.browsing_context.navigate(
            context=context,
            url=f"{fixture}/webdriver/basic?selenium-script-handles",
            wait=ReadinessState.COMPLETE,
        )

        context_realms = driver.script._get_realms(context=context)
        assert_true(
            any(
                realm.context == context and realm.type == RealmType.WINDOW
                for realm in context_realms
            ),
            "Selenium BiDi getRealms context window realm",
        )
        window_realms = driver.script._get_realms(type=RealmType.WINDOW)
        assert_true(
            any(realm.context == context for realm in window_realms),
            "Selenium BiDi getRealms type filter includes context",
        )

        owned = driver.script._evaluate(
            "({ foo: 'bar', count: 41 })",
            {"context": context},
            await_promise=False,
            result_ownership=ResultOwnership.ROOT,
        )
        assert_equal(owned.result["type"], "object", "Selenium BiDi root-owned result type")
        handle = owned.result.get("handle")
        assert_true(isinstance(handle, str) and handle, "Selenium BiDi root-owned result handle")

        serialized = driver.script._evaluate(
            "({ foo: 'bar' })",
            {"context": context},
            await_promise=False,
            result_ownership=ResultOwnership.NONE,
        )
        assert_true("handle" not in serialized.result, "Selenium BiDi none-owned result omits handle")

        argument_call = driver.script._call_function(
            "(obj) => obj.foo + ':' + (obj.count + 1)",
            await_promise=False,
            target={"context": context},
            arguments=[{"handle": handle}],
        )
        assert_equal(
            argument_call.result["value"],
            "bar:42",
            "Selenium BiDi callFunction uses handle argument",
        )

        this_call = driver.script._call_function(
            "function() { return this.count + 1; }",
            await_promise=False,
            target={"context": context},
            this={"handle": handle},
        )
        assert_equal(
            this_call.result["value"],
            42,
            "Selenium BiDi callFunction uses handle this",
        )

        driver.script._disown(handles=[handle], target={"context": context})
        try:
            driver.script._call_function(
                "(obj) => obj.foo",
                await_promise=False,
                target={"context": context},
                arguments=[{"handle": handle}],
            )
        except WebDriverException as error:
            assert_true("handle" in str(error).lower(), "Selenium BiDi disowned handle error")
        else:
            raise AssertionError("Selenium BiDi disowned handle should not remain usable")
    finally:
        driver.browsing_context.close(context)

    record(results, "selenium_bidi_script_handles", {"context": context})


def _run_selenium_bidi_script_serialization_smoke(
    driver: webdriver.Remote,
    fixture: str,
    results: list[dict[str, Any]],
) -> None:
    # Reduced from Selenium Python bidi_script_tests.py evaluate/callFunction
    # cases and Playwright page-evaluate.spec.ts unserializable value roundtrips.
    context = driver.browsing_context.create(type=WindowTypes.TAB)
    try:
        driver.browsing_context.navigate(
            context=context,
            url=f"{fixture}/webdriver/basic?selenium-script-serialization",
            wait=ReadinessState.COMPLETE,
        )

        for expression, expected in [
            ("NaN", "NaN"),
            ("-0", "-0"),
            ("Infinity", "Infinity"),
            ("-Infinity", "-Infinity"),
        ]:
            result = driver.script._evaluate(
                expression,
                {"context": context},
                await_promise=False,
            )
            assert_equal(
                result.result,
                {"type": "number", "value": expected},
                f"Selenium BiDi evaluate serializes {expression}",
            )

        bigint_result = driver.script._evaluate(
            "42n",
            {"context": context},
            await_promise=False,
        )
        assert_equal(
            bigint_result.result,
            {"type": "bigint", "value": "42"},
            "Selenium BiDi evaluate serializes BigInt",
        )

        direct_arguments = driver.script._call_function(
            (
                "(nan, negativeZero, infinity, negativeInfinity, big) => "
                "JSON.stringify({"
                "nan:Object.is(nan, NaN),"
                "negativeZero:Object.is(negativeZero, -0),"
                "infinity:Object.is(infinity, Infinity),"
                "negativeInfinity:Object.is(negativeInfinity, -Infinity),"
                "big:typeof big + ':' + String(big)"
                "})"
            ),
            await_promise=False,
            target={"context": context},
            arguments=[
                {"type": "number", "value": "NaN"},
                {"type": "number", "value": "-0"},
                {"type": "number", "value": "Infinity"},
                {"type": "number", "value": "-Infinity"},
                {"type": "bigint", "value": "17"},
            ],
        )
        assert_equal(
            json.loads(direct_arguments.result["value"]),
            {
                "nan": True,
                "negativeZero": True,
                "infinity": True,
                "negativeInfinity": True,
                "big": "bigint:17",
            },
            "Selenium BiDi callFunction deserializes direct special values",
        )

        owned = driver.script._evaluate(
            "({ label: 'nested-handle', value: 9 })",
            {"context": context},
            await_promise=False,
            result_ownership=ResultOwnership.ROOT,
        )
        handle = owned.result.get("handle")
        assert_true(isinstance(handle, str) and handle, "Selenium BiDi nested serialization handle")
        nested_arguments = driver.script._call_function(
            (
                "(payload) => JSON.stringify({"
                "label:payload.item.label,"
                "value:payload.item.value,"
                "same:payload.item === payload.again,"
                "nan:Object.is(payload.special.nan, NaN),"
                "negativeZero:Object.is(payload.special.negativeZero, -0),"
                "big:typeof payload.special.big + ':' + String(payload.special.big)"
                "})"
            ),
            await_promise=False,
            target={"context": context},
            arguments=[
                {
                    "type": "object",
                    "value": [
                        ["item", {"handle": handle}],
                        ["again", {"handle": handle}],
                        [
                            "special",
                            {
                                "type": "object",
                                "value": [
                                    ["nan", {"type": "number", "value": "NaN"}],
                                    ["negativeZero", {"type": "number", "value": "-0"}],
                                    ["big", {"type": "bigint", "value": "23"}],
                                ],
                            },
                        ],
                    ],
                }
            ],
        )
        assert_true(
            nested_arguments.result is not None,
            f"Selenium BiDi nested serialization result: {nested_arguments.exception_details}",
        )
        assert_equal(
            json.loads(nested_arguments.result["value"]),
            {
                "label": "nested-handle",
                "value": 9,
                "same": True,
                "nan": True,
                "negativeZero": True,
                "big": "bigint:23",
            },
            "Selenium BiDi callFunction deserializes nested handles and special values",
        )
        driver.script._disown(handles=[handle], target={"context": context})
    finally:
        driver.browsing_context.close(context)

    record(results, "selenium_bidi_script_serialization", {"context": context})


def _run_selenium_bidi_network_handler_smoke(
    driver: webdriver.Remote,
    fixture: str,
    results: list[dict[str, Any]],
) -> None:
    # Reduced from Selenium Python bidi_network_tests.py. Selenium's helper
    # intentionally registers the intercept before subscribing the callback.
    seen: list[str] = []
    blocked_seen: list[bool] = []
    exceptions: list[str] = []
    data_url = f"{fixture}/webdriver/network-data"

    def on_before_request(request: Any) -> None:
        try:
            raw = getattr(request, "_params", {}) or {}
            if request.url == data_url:
                seen.append(request.url)
                blocked_seen.append(raw.get("isBlocked") is True and bool(raw.get("intercepts")))
                if raw.get("isBlocked") or raw.get("intercepts"):
                    request.continue_request()
        except Exception as error:  # pragma: no cover - surfaced below.
            exceptions.append(str(error))

    context = driver.browsing_context.create(type=WindowTypes.TAB)

    def network_body() -> str | bool:
        result = driver.script.evaluate(
            "globalThis.__lmSeleniumNetworkHandlerBody",
            {"context": context},
            False,
        )
        value = result["result"].get("value")
        if value == "pending":
            return False
        return value or False

    try:
        driver.browsing_context.navigate(
            context=context,
            url=f"{fixture}/webdriver/basic",
            wait=ReadinessState.COMPLETE,
        )
        callback_id = driver.network.add_request_handler("before_request", on_before_request)
        assert_true(callback_id is not None, "Selenium BiDi network handler id")
        try:
            fetch_script = (
                "globalThis.__lmSeleniumNetworkHandlerBody = 'pending';"
                "fetch("
                + json.dumps(data_url)
                + ', { method: "POST", body: "webdriver request body" })'
                + ".then(response => response.text())"
                + ".then(text => { globalThis.__lmSeleniumNetworkHandlerBody = text; })"
                + ".catch(error => { globalThis.__lmSeleniumNetworkHandlerBody = String(error); });"
                + "'scheduled'"
            )
            fetch_result = driver.script.evaluate(
                fetch_script,
                {"context": context},
                False,
            )
            assert_equal(
                fetch_result["result"]["value"],
                "scheduled",
                "Selenium BiDi network handler scheduled fetch",
            )
            WebDriverWait(driver, 5, poll_frequency=0.05).until(lambda _driver: bool(seen))
            assert_true(
                any(blocked_seen),
                "Selenium BiDi network handler should receive a blocked target request",
            )
            body = WebDriverWait(driver, 5, poll_frequency=0.05).until(
                lambda _driver: network_body()
            )
            assert_equal(
                body,
                "webdriver network body",
                "Selenium BiDi network handler continued fetch body",
            )
            assert_equal(exceptions, [], "Selenium BiDi network handler callback exceptions")
        finally:
            driver.network.remove_request_handler("before_request", callback_id)
    finally:
        driver.browsing_context.close(context)

    record(results, "selenium_bidi_network_handler", {"context": context, "url": data_url})


def _run_selenium_bidi_network_auth_handler_smoke(
    driver: webdriver.Remote,
    fixture: str,
    results: list[dict[str, Any]],
) -> None:
    # Reduced from Selenium Python bidi_network_tests.py test_continue_with_auth.
    context = driver.browsing_context.create(type=WindowTypes.TAB)
    auth_url = f"{fixture}/webdriver/basic-auth"
    callback_id = None
    try:
        callback_id = driver.network.add_auth_handler("postman", "password")
        assert_true(callback_id is not None, "Selenium BiDi auth handler id")

        navigation = driver.browsing_context.navigate(
            context=context,
            url=auth_url,
            wait=ReadinessState.COMPLETE,
        )
        assert_equal(navigation["url"], auth_url, "Selenium BiDi auth handler navigation URL")
        body = driver.script.evaluate(
            "document.body.textContent",
            {"context": context},
            False,
        )
        assert_true(
            "authenticated" in body["result"]["value"],
            "Selenium BiDi auth handler authenticated body",
        )
    finally:
        if callback_id is not None:
            driver.network.remove_auth_handler(callback_id)
        driver.browsing_context.close(context)

    assert_equal(driver.network.intercepts, [], "Selenium BiDi auth handler removes intercept")
    record(results, "selenium_bidi_network_auth_handler", {"context": context, "url": auth_url})


def _run_selenium_bidi_browser_emulation_smoke(
    driver: webdriver.Remote,
    fixture: str,
    results: list[dict[str, Any]],
) -> None:
    # Reduced from Selenium's bidi_browser_tests.py and bidi_emulation_tests.py.
    user_context = driver.browser.create_user_context()
    assert_true(isinstance(user_context, str) and user_context, "Selenium BiDi browser userContext")
    try:
        user_contexts = driver.browser.get_user_contexts()
        assert_true(user_context in user_contexts, "Selenium BiDi browser get_user_contexts includes created context")

        user_agent = "MoliSeleniumFacade/1.0"
        locale = "fr-FR"
        timezone = "Asia/Tokyo"
        driver.emulation.set_user_agent_override(user_agent, user_contexts=[user_context])
        driver.emulation.set_locale_override(locale, user_contexts=[user_context])
        driver.emulation.set_timezone_override(timezone, user_contexts=[user_context])
        driver.browsing_context.set_viewport(
            user_contexts=[user_context],
            viewport={"width": 420, "height": 260},
            device_pixel_ratio=2,
        )

        context = driver.browsing_context.create(type=WindowTypes.TAB, user_context=user_context)
        assert_true(isinstance(context, str) and context, "Selenium BiDi browser custom context")
        try:
            client_windows = driver.browser.get_client_windows()
            assert_true(client_windows, "Selenium BiDi browser get_client_windows returns at least one window")
            assert_true(
                any(isinstance(window.client_window, str) and window.client_window for window in client_windows),
                "Selenium BiDi browser client window id",
            )

            page_url = f"{fixture}/webdriver/profile-echo"
            navigation = driver.browsing_context.navigate(
                context=context,
                url=page_url,
                wait=ReadinessState.COMPLETE,
            )
            assert_equal(navigation["url"], page_url, "Selenium BiDi emulation navigate URL")
            profile_result = driver.script.evaluate(
                (
                    "JSON.stringify({"
                    "userAgent:navigator.userAgent,"
                    "language:navigator.language,"
                    "languages:navigator.languages,"
                    "locale:Intl.DateTimeFormat().resolvedOptions().locale,"
                    "timeZone:Intl.DateTimeFormat().resolvedOptions().timeZone,"
                    "width:innerWidth,"
                    "height:innerHeight,"
                    "dpr:devicePixelRatio,"
                    "headerEcho:JSON.parse(document.getElementById('profile-echo').textContent)"
                    "})"
                ),
                {"context": context},
                False,
            )
            profile = json.loads(profile_result["result"]["value"])
            assert_equal(profile["userAgent"], user_agent, "Selenium BiDi emulation navigator.userAgent")
            assert_equal(profile["language"], locale, "Selenium BiDi emulation navigator.language")
            assert_equal(profile["languages"][0], locale, "Selenium BiDi emulation navigator.languages")
            assert_equal(profile["locale"], locale, "Selenium BiDi emulation Intl locale")
            assert_equal(profile["timeZone"], timezone, "Selenium BiDi emulation timezone")
            assert_equal((profile["width"], profile["height"]), (420, 260), "Selenium BiDi emulation viewport")
            assert_equal(profile["dpr"], 2, "Selenium BiDi emulation devicePixelRatio")
            assert_equal(profile["headerEcho"]["userAgent"], user_agent, "Selenium BiDi emulation User-Agent header")
            assert_equal(
                profile["headerEcho"]["acceptLanguage"],
                locale,
                "Selenium BiDi emulation Accept-Language header",
            )
            assert_equal(
                driver.script.evaluate("navigator.onLine", {"context": context}, False)["result"]["value"],
                True,
                "Selenium BiDi emulation navigator.onLine default",
            )
            try:
                driver.emulation.set_network_conditions(offline=True, contexts=[context])
                assert_equal(
                    driver.script.evaluate("navigator.onLine", {"context": context}, False)["result"]["value"],
                    False,
                    "Selenium BiDi emulation context network offline",
                )
            finally:
                driver.emulation.set_network_conditions(offline=False, contexts=[context])
            assert_equal(
                driver.script.evaluate("navigator.onLine", {"context": context}, False)["result"]["value"],
                True,
                "Selenium BiDi emulation context network reset",
            )
            try:
                driver.emulation.set_network_conditions(offline=True, user_contexts=[user_context])
                assert_equal(
                    driver.script.evaluate("navigator.onLine", {"context": context}, False)["result"]["value"],
                    False,
                    "Selenium BiDi emulation userContext network offline",
                )
            finally:
                driver.emulation.set_network_conditions(offline=False, user_contexts=[user_context])
            assert_equal(
                driver.script.evaluate("navigator.onLine", {"context": context}, False)["result"]["value"],
                True,
                "Selenium BiDi emulation userContext network reset",
            )
        finally:
            driver.browsing_context.close(context)
    finally:
        driver.browser.remove_user_context(user_context)

    assert_true(
        user_context not in driver.browser.get_user_contexts(),
        "Selenium BiDi browser remove_user_context",
    )
    record(results, "selenium_bidi_browser_emulation", {"userContext": user_context})


def _run_selenium_navigation_element_script_smoke(
    driver: webdriver.Remote,
    fixture: str,
    results: list[dict[str, Any]],
) -> None:
    page_url = f"{fixture}/webdriver/basic"
    driver.get(page_url)
    assert_equal(driver.current_url, page_url, "Selenium current URL")
    assert_equal(driver.title, "WebDriver Smoke Basic", "Selenium title")

    main = driver.find_element(By.ID, "main")
    assert_equal(main.text, "Basic Ready", "Selenium element text")
    assert_equal(main.tag_name, "main", "Selenium element tag name")
    assert_equal(main.get_attribute("class"), "no cheese", "Selenium element class attribute")
    assert_true(main.is_displayed(), "Selenium displayed state")
    assert_true(main.is_enabled(), "Selenium enabled state")
    assert_true(isinstance(main.rect.get("width"), (int, float)), "Selenium element rect width")
    assert_true(isinstance(main.value_of_css_property("display"), str), "Selenium CSS property")
    assert_true("Basic Ready" in driver.page_source, "Selenium page source contains fixture text")

    field = driver.find_element(By.ID, "field")
    field.send_keys("typed")
    assert_equal(field.get_property("value"), "typed", "Selenium send keys value")
    field.clear()
    assert_equal(field.get_property("value"), "", "Selenium clear value")

    labelled = driver.find_element(By.ID, "labelled")
    echoed = driver.execute_script(
        "return arguments[0].textContent + ':' + arguments[1].getAttribute('aria-label');",
        main,
        labelled,
    )
    assert_equal(echoed, "Basic Ready:Smoke Label", "Selenium execute_script WebElement args")
    async_value = driver.execute_async_script(
        "const done = arguments[arguments.length - 1]; setTimeout(() => done('async ok'), 0);"
    )
    assert_equal(async_value, "async ok", "Selenium execute_async_script timer result")

    child_frame = driver.find_element(By.ID, "child")
    driver.switch_to.frame(child_frame)
    assert_equal(
        driver.find_element(By.ID, "inside-frame").text,
        "frame ready",
        "Selenium frame element text",
    )
    driver.switch_to.parent_frame()
    assert_equal(driver.find_element(By.ID, "main").text, "Basic Ready", "Selenium parent frame switch")
    driver.switch_to.frame(0)
    assert_equal(driver.find_element(By.ID, "inside-frame").text, "frame ready", "Selenium index frame switch")
    driver.switch_to.default_content()
    assert_equal(driver.find_element(By.LINK_TEXT, "Basic Link").tag_name, "a", "Selenium link text locator")
    assert_equal(
        driver.find_element(By.PARTIAL_LINK_TEXT, "Basic").get_attribute("id"),
        "relative-link",
        "Selenium partial link text locator",
    )

    record(
        results,
        "selenium_navigation_element_script",
        {"currentUrl": driver.current_url, "title": driver.title},
    )


def _run_selenium_script_return_and_pinning_smoke(
    driver: webdriver.Remote,
    fixture: str,
    results: list[dict[str, Any]],
) -> None:
    # Reduced from Selenium executing_javascript_tests.py and script_pinning_tests.py.
    driver.get(f"{fixture}/webdriver/basic")
    driver.execute_script(
        """
        document.body.innerHTML = `
          <main id="script-main">Script Ready</main>
          <nav class="navigation">
            <a id="first-link" href="#first">First</a>
            <a id="second-link" href="#second">Second</a>
          </nav>
        `;
        """
    )

    assert_equal(
        driver.execute_script("return document.title"),
        "WebDriver Smoke Basic",
        "Selenium script string result",
    )
    assert_equal(
        driver.execute_script("return document.querySelectorAll('nav a').length"),
        2,
        "Selenium script integer result",
    )
    assert_equal(driver.execute_script("return true"), True, "Selenium script boolean result")
    assert_equal(
        driver.execute_script("return ['zero', [true, false]]"),
        ["zero", [True, False]],
        "Selenium script nested array result",
    )

    main = driver.execute_script("return document.getElementById('script-main')")
    assert_true(isinstance(main, WebElement), "Selenium script returns WebElement")
    assert_equal(main.tag_name, "main", "Selenium returned WebElement tag")
    links = driver.execute_script("return document.querySelectorAll('nav a')")
    assert_equal(
        [link.get_attribute("id") for link in links],
        ["first-link", "second-link"],
        "Selenium script returns NodeList as WebElements",
    )
    nested_list = driver.execute_script("return [document.body, [document.getElementById('first-link')]]")
    assert_true(isinstance(nested_list[0], WebElement), "Selenium script nested list body element")
    assert_equal(nested_list[1][0].get_attribute("id"), "first-link", "Selenium script nested list link element")
    nested_dict = driver.execute_script(
        "return {el1: document.body, nested: {el2: document.getElementById('second-link')}, list: [document.getElementById('first-link')]}"
    )
    assert_true(isinstance(nested_dict["el1"], WebElement), "Selenium script dict body element")
    assert_equal(
        nested_dict["nested"]["el2"].get_attribute("id"),
        "second-link",
        "Selenium script nested dict element",
    )
    assert_equal(nested_dict["list"][0].get_attribute("id"), "first-link", "Selenium script dict list element")
    assert_equal(
        driver.execute_script("return arguments[0]", 1.25),
        1.25,
        "Selenium script decimal argument round trip",
    )
    assert_equal(
        driver.execute_script("return arguments[0].textContent + ':' + arguments[1]", main, "arg"),
        "Script Ready:arg",
        "Selenium script WebElement argument round trip",
    )

    with warnings.catch_warnings():
        warnings.simplefilter("ignore", DeprecationWarning)
        driver.pinned_scripts = {}
        first = driver.pin_script("return arguments[0] + ':pinned';")
        second = driver.pin_script("return document.getElementById('script-main').textContent;")
        assert_equal(driver.execute_script(first, "value"), "value:pinned", "Selenium pinned script argument")
        assert_equal(driver.execute_script(second), "Script Ready", "Selenium pinned script DOM access")
        assert_equal(
            driver.get_pinned_scripts(),
            [first.id, second.id],
            "Selenium pinned script listing",
        )
        driver.unpin(first)
        assert_equal(driver.get_pinned_scripts(), [second.id], "Selenium pinned script unpin")
        driver.unpin(second)
        try:
            driver.execute_script(first)
        except JavascriptException:
            pass
        else:
            raise AssertionError("Selenium unpinned script should raise JavascriptException")

    record(results, "selenium_script_return_and_pinning")


def _run_selenium_async_script_smoke(
    driver: webdriver.Remote,
    fixture: str,
    results: list[dict[str, Any]],
) -> None:
    # Reduced from Selenium executing_async_javascript_tests.py.
    driver.set_script_timeout(5)
    try:
        driver.get(f"{fixture}/webdriver/basic")
        driver.execute_script(
            """
            document.body.innerHTML = `
              <main id="async-main">Async Ready</main>
              <a id="async-link" href="#async">Async Link</a>
            `;
            """
        )

        assert_equal(
            driver.execute_async_script("arguments[arguments.length - 1](123);"),
            123,
            "Selenium async script integer result",
        )
        assert_equal(
            driver.execute_async_script("arguments[arguments.length - 1]('abc');"),
            "abc",
            "Selenium async script string result",
        )
        assert_equal(
            driver.execute_async_script("arguments[arguments.length - 1](false);"),
            False,
            "Selenium async script false result",
        )
        assert_equal(
            driver.execute_async_script("arguments[arguments.length - 1](true);"),
            True,
            "Selenium async script true result",
        )
        assert_equal(
            driver.execute_async_script("arguments[arguments.length - 1](null);"),
            None,
            "Selenium async script null result",
        )
        assert_equal(
            driver.execute_async_script("arguments[arguments.length - 1]();"),
            None,
            "Selenium async script undefined result",
        )
        assert_equal(
            driver.execute_async_script("arguments[arguments.length - 1]([null, 123, 'abc', true, false]);"),
            [None, 123, "abc", True, False],
            "Selenium async script primitive array result",
        )

        body = driver.execute_async_script("arguments[arguments.length - 1](document.body);")
        assert_true(isinstance(body, WebElement), "Selenium async script returns WebElement")
        assert_equal(body.tag_name, "body", "Selenium async returned body tag")

        returned = driver.execute_async_script(
            "arguments[arguments.length - 1]([document.getElementById('async-main'), [document.getElementById('async-link')]]);"
        )
        assert_equal(returned[0].tag_name, "main", "Selenium async nested WebElement main")
        assert_equal(returned[1][0].get_attribute("id"), "async-link", "Selenium async nested WebElement link")

        assert_equal(
            driver.execute_async_script(
                "arguments[arguments.length - 1](arguments[0] + arguments[1]);",
                1,
                2,
            ),
            3,
            "Selenium async script multiple primitive arguments",
        )
        main = driver.find_element(By.ID, "async-main")
        assert_equal(
            driver.execute_async_script(
                "arguments[arguments.length - 1](arguments[0].textContent + ':' + arguments[1]);",
                main,
                "arg",
            ),
            "Async Ready:arg",
            "Selenium async script WebElement argument",
        )

        _assert_raises(
            WebDriverException,
            lambda: driver.execute_async_script("throw Error('async exploded');"),
            "Selenium async script initial exception",
        )
        driver.set_script_timeout(0.1)
        _assert_raises(
            TimeoutException,
            lambda: driver.execute_async_script("setTimeout(arguments[arguments.length - 1], 500);"),
            "Selenium async script timeout",
        )
        driver.set_script_timeout(5)
        assert_equal(
            driver.execute_async_script("arguments[arguments.length - 1]('async recovered');"),
            "async recovered",
            "Selenium async script recovers after timeout",
        )
    finally:
        driver.set_script_timeout(30)

    record(results, "selenium_async_script")


def _run_selenium_text_handling_smoke(
    driver: webdriver.Remote,
    fixture: str,
    results: list[dict[str, Any]],
) -> None:
    # Reduced from Selenium text_handling_tests.py and the legacy atoms text tests.
    driver.get(f"{fixture}/webdriver/basic")
    driver.execute_script(
        """
        document.body.innerHTML = `
          <p id="oneline">A single line of text</p>
          <p id="hiddenline" style="visibility: hidden">A hidden line of text</p>
          <div id="multiline">
            <p>A div containing</p>
            More than one line of text<br>
            <div>and block level elements</div>
          </div>
          <span id="span">An inline element</span>
          <p id="lotsofspaces">This line has lots

              of spaces.
          </p>
          <p id="nbsp">This line has a&nbsp;non-breaking space</p>
          <p id="nbspandspaces">This line has a &nbsp; non-breaking space and spaces</p>
          <p id="inline">This <span id="inlinespan">    line has <em>text</em>\t</span> within elements that are meant to be displayed inline</p>
          <div id="twoblocks"><p>Some text</p><p>Some more text</p></div>
          <label id="labelforusername" for="username">
            Username: <input id="username" type="text" name="username">
            <script>document.getElementById('username').value = 'Michael';</script>
          </label>
          <div id="visible-wrapper">visible <span style="display: none">hidden</span><span>text</span></div>
          <div id="empty"></div>
          <p id="spaces">    </p>
        `;
        """
    )

    assert_equal(
        driver.find_element(By.ID, "oneline").text,
        "A single line of text",
        "Selenium text single element",
    )
    assert_equal(
        driver.find_element(By.ID, "multiline").text,
        "A div containing\nMore than one line of text\nand block level elements",
        "Selenium text includes block-level line breaks",
    )
    assert_equal(
        driver.find_element(By.ID, "lotsofspaces").text,
        "This line has lots of spaces.",
        "Selenium text collapses whitespace",
    )
    assert_equal(
        driver.find_element(By.ID, "nbsp").text,
        "This line has a non-breaking space",
        "Selenium text converts NBSP",
    )
    assert_equal(
        driver.find_element(By.ID, "nbspandspaces").text,
        "This line has a   non-breaking space and spaces",
        "Selenium text preserves NBSP-adjacent spacing",
    )
    assert_equal(
        driver.find_element(By.ID, "inline").text,
        "This line has text within elements that are meant to be displayed inline",
        "Selenium text handles inline descendants",
    )
    assert_equal(
        driver.find_element(By.ID, "inlinespan").text,
        "line has text",
        "Selenium text handles inline child whitespace",
    )
    assert_equal(
        driver.find_element(By.ID, "span").text,
        "An inline element",
        "Selenium text returns inline element content",
    )
    assert_equal(
        driver.find_element(By.ID, "twoblocks").text,
        "Some text\nSome more text",
        "Selenium text separates sibling blocks",
    )
    assert_equal(
        driver.find_element(By.ID, "labelforusername").text,
        "Username:",
        "Selenium text ignores script elements",
    )
    assert_equal(
        driver.find_element(By.ID, "visible-wrapper").text,
        "visible text",
        "Selenium text excludes display:none descendants",
    )
    assert_equal(driver.find_element(By.ID, "hiddenline").text, "", "Selenium text excludes hidden elements")
    assert_equal(driver.find_element(By.ID, "empty").text, "", "Selenium text empty element")
    assert_equal(driver.find_element(By.ID, "spaces").text, "", "Selenium text only spaces")

    record(results, "selenium_text_handling")


def _run_selenium_locator_accessibility_smoke(
    driver: webdriver.Remote,
    fixture: str,
    results: list[dict[str, Any]],
) -> None:
    # Reduced from Selenium element_aria_label_tests.py / element_aria_tests.py
    # and paired with Playwright's user-facing getBy* locator coverage.
    driver.get(f"{fixture}/webdriver/basic")
    driver.execute_script(
        """
        document.body.innerHTML = `
          <section id="selectors">
            <h1 id="heading">Level 1 Header</h1>
            <div data-testid="Hello">Hello world</div>
            <label for="first-input">First Name</label>
            <input id="first-input" type="text">
            <label for="last-input">Last <span>Name</span></label>
            <input id="last-input" type="text">
            <label id="launch-label">Launch</label>
            <button id="labelled-button" aria-labelledby="launch-label"><span>Click me</span></button>
            <h2 id="explicit-alert" role="alert">Explicit Alert</h2>
            <a id="accessible-link" href="/webdriver/basic">Accessible Link</a>
            <img id="logo" alt="Logo Alt">
            <label for="bio">Biography</label>
            <textarea id="bio"></textarea>
            <label for="food">Favorite Food</label>
            <select id="food"><option>Pizza</option></select>
            <input id="submit-input" type="submit" value="Send Form">
            <label><input id="subscribe" type="checkbox"> Subscribe</label>
            <input id="aria-input" aria-label="Secret Code">
            <input id="placeholder-one" placeholder="Hello">
            <input id="placeholder-two" placeholder="Hello World">
            <iframe id="child" srcdoc="<button id='frame-button' aria-label='Frame Launch'>Frame button</button>"></iframe>
          </section>
        `;
        """
    )

    heading = driver.find_element(By.ID, "heading")
    assert_equal(heading.accessible_name, "Level 1 Header", "Selenium accessible_name from text")
    assert_equal(heading.aria_role, "heading", "Selenium implicit aria_role")
    assert_equal(
        driver.find_element(By.ID, "explicit-alert").aria_role,
        "alert",
        "Selenium explicit aria_role overrides tag",
    )

    assert_equal(
        driver.find_element(By.CSS_SELECTOR, "[data-testid='Hello']").text,
        "Hello world",
        "Selenium CSS data-testid locator",
    )
    first_label = driver.find_element(By.XPATH, "//label[normalize-space(.)='First Name']")
    first = driver.find_element(By.ID, first_label.get_attribute("for"))
    first.send_keys("Ada")
    assert_equal(first.get_property("value"), "Ada", "Selenium label for-associated input")
    assert_equal(
        driver.find_element(By.XPATH, "//label[contains(normalize-space(.), 'Last Name')]").get_attribute("for"),
        "last-input",
        "Selenium nested label text lookup",
    )
    assert_equal(
        driver.find_element(By.CSS_SELECTOR, "[aria-label='Secret Code']").get_attribute("id"),
        "aria-input",
        "Selenium CSS aria-label locator",
    )
    assert_equal(
        driver.find_element(By.CSS_SELECTOR, "input[placeholder='Hello']").get_attribute("id"),
        "placeholder-one",
        "Selenium CSS placeholder locator",
    )

    labelled_button = driver.find_element(By.ID, "labelled-button")
    assert_equal(
        labelled_button.accessible_name,
        "Launch",
        "Selenium accessible_name from aria-labelledby",
    )
    assert_equal(labelled_button.aria_role, "button", "Selenium button implicit aria_role")
    assert_equal(
        driver.find_element(By.ID, "accessible-link").accessible_name,
        "Accessible Link",
        "Selenium accessible_name from link text",
    )
    assert_equal(driver.find_element(By.ID, "accessible-link").aria_role, "link", "Selenium link aria_role")
    assert_equal(driver.find_element(By.ID, "logo").accessible_name, "Logo Alt", "Selenium img alt accessible_name")
    assert_equal(driver.find_element(By.ID, "logo").aria_role, "img", "Selenium img aria_role")
    assert_equal(driver.find_element(By.ID, "bio").accessible_name, "Biography", "Selenium textarea label")
    assert_equal(driver.find_element(By.ID, "bio").aria_role, "textbox", "Selenium textarea aria_role")
    assert_equal(driver.find_element(By.ID, "food").accessible_name, "Favorite Food", "Selenium select label")
    assert_equal(driver.find_element(By.ID, "food").aria_role, "combobox", "Selenium select aria_role")
    assert_equal(driver.find_element(By.ID, "submit-input").accessible_name, "Send Form", "Selenium submit value label")
    assert_equal(driver.find_element(By.ID, "submit-input").aria_role, "button", "Selenium submit aria_role")
    assert_equal(driver.find_element(By.ID, "subscribe").accessible_name, "Subscribe", "Selenium checkbox label")
    assert_equal(driver.find_element(By.ID, "subscribe").aria_role, "checkbox", "Selenium checkbox aria_role")

    child_frame = driver.find_element(By.ID, "child")
    driver.switch_to.frame(child_frame)
    frame_button = driver.find_element(By.CSS_SELECTOR, "button[aria-label='Frame Launch']")
    assert_equal(frame_button.text, "Frame button", "Selenium frame scoped aria-label locator")
    assert_equal(frame_button.accessible_name, "Frame Launch", "Selenium frame accessible_name")
    driver.switch_to.default_content()

    record(results, "selenium_locator_accessibility")


def _run_selenium_relative_locator_smoke(
    driver: webdriver.Remote,
    fixture: str,
    results: list[dict[str, Any]],
) -> None:
    # Reduced from Selenium Python support/relative_by_tests.py. The Selenium
    # client injects findElements.js via execute_script and filters by
    # getBoundingClientRect(), so this covers client-side relative locators plus
    # Moli's WebElement argument/result round trip.
    driver.get(f"{fixture}/webdriver/basic")
    driver.execute_script(
        """
        document.body.innerHTML = `
          <p id="above">above</p>
          <p id="mid">mid</p>
          <p id="below">below</p>
          <table>
            <tbody id="relative-grid">
              <tr>
                <td id="topLeft">top left</td>
                <td id="top">top</td>
                <td id="topRight">top right</td>
              </tr>
              <tr>
                <td id="left">left</td>
                <td id="center">center</td>
                <td id="right">right</td>
              </tr>
              <tr>
                <td id="bottomLeft">bottom left</td>
                <td id="bottom">bottom</td>
                <td id="bottomRight">bottom right</td>
              </tr>
            </tbody>
          </table>
          <div id="rect-row">
            <div id="rect1">rect 1</div>
            <div id="rect2">rect 2</div>
            <div id="rect4">rect 4</div>
          </div>
        `;
        const assignRect = (id, left, top, width = 50, height = 30) => {
          const element = document.getElementById(id);
          const rect = {
            x: left,
            y: top,
            left,
            top,
            right: left + width,
            bottom: top + height,
            width,
            height,
            toJSON() { return this; },
          };
          element.getBoundingClientRect = () => rect;
          element.getClientRects = () => [rect];
        };
        assignRect("above", 0, 0);
        assignRect("mid", 0, 40);
        assignRect("below", 0, 80);
        assignRect("topLeft", 0, 140);
        assignRect("top", 70, 140);
        assignRect("topRight", 140, 140);
        assignRect("left", 0, 190);
        assignRect("center", 70, 190);
        assignRect("right", 140, 190);
        assignRect("bottomLeft", 0, 240);
        assignRect("bottom", 70, 240);
        assignRect("bottomRight", 140, 240);
        assignRect("rect1", 0, 320);
        assignRect("rect2", 60, 320);
        assignRect("rect4", 180, 320);
        """
    )

    lowest = driver.find_element(By.ID, "below")
    assert_equal(
        driver.find_element(locate_with(By.TAG_NAME, "p").above(lowest)).get_attribute("id"),
        "mid",
        "Selenium relative locator above WebElement",
    )
    assert_equal(
        driver.find_element(locate_with(By.TAG_NAME, "p").above({By.ID: "below"})).get_attribute("id"),
        "mid",
        "Selenium relative locator above locator dict",
    )
    above_ids = {
        element.get_attribute("id") for element in driver.find_elements(locate_with(By.TAG_NAME, "p").above(lowest))
    }
    assert_true({"above", "mid"}.issubset(above_ids), "Selenium relative locator find_elements above")

    combined = driver.find_elements(
        locate_with(By.CSS_SELECTOR, "td")
        .above(driver.find_element(By.ID, "center"))
        .to_right_of(driver.find_element(By.ID, "top"))
    )
    combined_ids = {element.get_attribute("id") for element in combined}
    assert_true("topRight" in combined_ids, "Selenium relative locator combines above and right")

    xpath_filtered = driver.find_elements(
        locate_with(By.XPATH, "//td[1]")
        .below(driver.find_element(By.ID, "top"))
        .above(driver.find_element(By.ID, "bottomLeft"))
    )
    xpath_ids = {element.get_attribute("id") for element in xpath_filtered}
    assert_true("left" in xpath_ids, "Selenium relative locator uses XPath root")

    assert_equal(
        driver.find_element(locate_with(By.ID, "rect2").near(driver.find_element(By.ID, "rect1"))).get_attribute("id"),
        "rect2",
        "Selenium relative locator near default distance",
    )
    _assert_raises(
        NoSuchElementException,
        lambda: driver.find_element(locate_with(By.ID, "rect4").near(driver.find_element(By.ID, "rect2"))),
        "Selenium relative locator near missing far element",
    )
    assert_equal(
        driver.find_element(locate_with(By.ID, "rect4").near({By.ID: "rect2"}, 120)).get_attribute("id"),
        "rect4",
        "Selenium relative locator near custom distance",
    )

    def ids_for(relative_by: object) -> set[str]:
        return {element.get_attribute("id") for element in driver.find_elements(relative_by)}

    assert_true(
        {"top", "topLeft", "topRight"}.issubset(ids_for(locate_with(By.TAG_NAME, "td").above({By.ID: "center"}))),
        "Selenium relative locator td above center",
    )
    assert_true(
        {"bottom", "bottomLeft", "bottomRight"}.issubset(ids_for(locate_with(By.TAG_NAME, "td").below({By.ID: "center"}))),
        "Selenium relative locator td below center",
    )
    assert_true(
        {"left", "topLeft", "bottomLeft"}.issubset(
            ids_for(locate_with(By.TAG_NAME, "td").to_left_of({By.ID: "center"}))
        ),
        "Selenium relative locator td left of center",
    )
    assert_true(
        {"right", "topRight", "bottomRight"}.issubset(
            ids_for(locate_with(By.TAG_NAME, "td").to_right_of({By.ID: "center"}))
        ),
        "Selenium relative locator td right of center",
    )

    _assert_raises(
        NoSuchElementException,
        lambda: driver.find_element(locate_with(By.ID, "not-present").above({By.ID: "top"})),
        "Selenium relative locator missing root raises NoSuchElementException",
    )

    record(results, "selenium_relative_locator")


def _run_selenium_network_storage_smoke(
    driver: webdriver.Remote,
    fixture: str,
    results: list[dict[str, Any]],
) -> None:
    driver.get(f"{fixture}/webdriver/basic")
    data_url = f"{fixture}/webdriver/network-data"

    fetch_get = driver.execute_async_script(
        """
        const url = arguments[0];
        const done = arguments[arguments.length - 1];
        fetch(url)
          .then(response => response.text())
          .then(text => done({ ok: true, text }))
          .catch(error => done({ ok: false, error: String(error) }));
        """,
        data_url,
    )
    assert_equal(
        fetch_get,
        {"ok": True, "text": "webdriver network body"},
        "Selenium execute_async_script fetch GET",
    )

    fetch_post = driver.execute_async_script(
        """
        const url = arguments[0];
        const done = arguments[arguments.length - 1];
        fetch(url, { method: "POST", body: "webdriver request body" })
          .then(response => response.text())
          .then(text => done({ ok: true, text }))
          .catch(error => done({ ok: false, error: String(error) }));
        """,
        data_url,
    )
    assert_equal(
        fetch_post,
        {"ok": True, "text": "webdriver network body"},
        "Selenium execute_async_script fetch POST",
    )

    xhr_post = driver.execute_async_script(
        """
        const url = arguments[0];
        const done = arguments[arguments.length - 1];
        const xhr = new XMLHttpRequest();
        xhr.open("POST", url);
        xhr.onload = () => done({ phase: "load", status: xhr.status, text: xhr.responseText });
        xhr.onerror = () => done({ phase: "error", status: xhr.status, text: xhr.responseText });
        xhr.send("webdriver request body");
        """,
        data_url,
    )
    assert_equal(
        xhr_post,
        {"phase": "load", "status": 200, "text": "webdriver network body"},
        "Selenium execute_async_script XHR POST",
    )

    storage = driver.execute_script(
        """
        localStorage.clear();
        sessionStorage.clear();
        localStorage.setItem("selenium-local", "local-value");
        sessionStorage.setItem("selenium-session", "session-value");
        return JSON.stringify({
          local: localStorage.getItem("selenium-local"),
          session: sessionStorage.getItem("selenium-session"),
        });
        """
    )
    assert_equal(
        storage,
        '{"local":"local-value","session":"session-value"}',
        "Selenium Web Storage set/read",
    )
    driver.refresh()
    storage_after_reload = driver.execute_script(
        """
        return JSON.stringify({
          local: localStorage.getItem("selenium-local"),
          session: sessionStorage.getItem("selenium-session"),
        });
        """
    )
    assert_equal(
        storage_after_reload,
        '{"local":"local-value","session":"session-value"}',
        "Selenium Web Storage persists across reload",
    )

    indexeddb = driver.execute_async_script(
        """
        const done = arguments[arguments.length - 1];
        const name = `selenium-smoke-db-${Date.now()}-${Math.random()}`;
        const request = indexedDB.open(name, 1);
        request.onupgradeneeded = () => request.result.createObjectStore("store");
        request.onerror = () => done({ ok: false, phase: "open", error: String(request.error) });
        request.onsuccess = () => {
          const db = request.result;
          const writeTx = db.transaction("store", "readwrite");
          writeTx.objectStore("store").put("indexed-value", "key");
          writeTx.onerror = () => {
            db.close();
            done({ ok: false, phase: "write", error: String(writeTx.error) });
          };
          writeTx.oncomplete = () => {
            const readTx = db.transaction("store", "readonly");
            const read = readTx.objectStore("store").get("key");
            read.onerror = () => {
              db.close();
              done({ ok: false, phase: "read", error: String(read.error) });
            };
            read.onsuccess = () => {
              const value = read.result;
              db.close();
              done({ ok: true, value });
            };
          };
        };
        """
    )
    assert_equal(
        indexeddb,
        {"ok": True, "value": "indexed-value"},
        "Selenium IndexedDB put/get",
    )

    record(results, "selenium_network_storage")


def _run_selenium_remote_downloads_smoke(
    driver: webdriver.Remote,
    fixture: str,
    results: list[dict[str, Any]],
) -> None:
    driver.get(f"{fixture}/webdriver/download-page")
    driver.find_element(By.ID, "download-one").click()
    driver.find_element(By.ID, "download-two").click()
    WebDriverWait(driver, 5, poll_frequency=0.05).until(
        lambda active_driver: "file_2.jpg" in active_driver.get_downloadable_files()
    )
    downloadable_files = driver.get_downloadable_files()
    assert_true("file_1.txt" in downloadable_files, "Selenium downloadable files includes text file")
    assert_true("file_2.jpg" in downloadable_files, "Selenium downloadable files includes image file")

    with TemporaryDirectory(prefix="moli-webdriver-selenium-download-") as tempdir:
        driver.download_file("file_1.txt", tempdir)
        downloaded = Path(tempdir) / "file_1.txt"
        assert_equal(
            downloaded.read_text(encoding="utf-8"),
            "Hello, World!",
            "Selenium remote download_file contents",
        )

    driver.delete_downloadable_files()
    assert_equal(driver.get_downloadable_files(), [], "Selenium delete downloadable files")
    driver.get(f"{fixture}/webdriver/basic")
    record(results, "selenium_remote_downloads")


def _run_selenium_cookie_management_smoke(
    driver: webdriver.Remote,
    fixture: str,
    results: list[dict[str, Any]],
) -> None:
    # Reduced from Selenium's cookie_tests.py.
    driver.get(f"{fixture}/webdriver/cookie-echo")
    cookie_domain = driver.execute_script("return location.hostname;")
    base_cookie = {
        "name": "seleniumCookie",
        "value": "base",
        "domain": cookie_domain,
        "path": "/",
        "secure": False,
    }

    driver.delete_all_cookies()
    assert_equal(driver.get_cookies(), [], "Selenium delete_all_cookies starts clean")

    driver.add_cookie(base_cookie)
    document_cookie = driver.execute_script("return document.cookie;")
    assert_true("seleniumCookie=base" in document_cookie, "Selenium add_cookie updates document.cookie")
    assert_equal(
        driver.get_cookie("seleniumCookie")["value"],
        "base",
        "Selenium get_cookie by name",
    )

    driver.add_cookie(
        {
            **base_cookie,
            "name": "seleniumStrict",
            "value": "strict",
            "sameSite": "Strict",
        }
    )
    assert_equal(
        driver.get_cookie("seleniumStrict")["sameSite"],
        "Strict",
        "Selenium SameSite Strict cookie",
    )
    driver.add_cookie(
        {
            **base_cookie,
            "name": "seleniumLax",
            "value": "lax",
            "sameSite": "Lax",
        }
    )
    assert_equal(
        driver.get_cookie("seleniumLax")["sameSite"],
        "Lax",
        "Selenium SameSite Lax cookie",
    )

    driver.delete_all_cookies()
    expired_cookie = {
        **base_cookie,
        "name": "seleniumExpired",
        "expiry": int(time.time()) - 1,
    }
    driver.add_cookie(expired_cookie)
    assert_equal(driver.get_cookie("seleniumExpired"), None, "Selenium expired cookie lookup")
    assert_equal(driver.get_cookies(), [], "Selenium expired cookie keeps store empty")

    driver.add_cookie({**base_cookie, "name": "seleniumSimilar", "value": "first"})
    driver.add_cookie({**base_cookie, "name": "seleniumSimilarx", "value": "second"})
    driver.delete_cookie("seleniumSimilar")
    similar_cookies = {cookie["name"]: cookie["value"] for cookie in driver.get_cookies()}
    assert_true("seleniumSimilar" not in similar_cookies, "Selenium delete_cookie removes exact name")
    assert_equal(
        similar_cookies.get("seleniumSimilarx"),
        "second",
        "Selenium delete_cookie preserves similar name",
    )

    for empty_name in ("", "   ", None):
        try:
            driver.get_cookie(empty_name)  # type: ignore[arg-type]
        except ValueError:
            pass
        else:
            raise AssertionError(f"Selenium get_cookie should reject empty name {empty_name!r}")

    for empty_name in ("", "   ", None):
        try:
            driver.delete_cookie(empty_name)  # type: ignore[arg-type]
        except ValueError:
            pass
        else:
            raise AssertionError(f"Selenium delete_cookie should reject empty name {empty_name!r}")

    driver.delete_all_cookies()
    assert_equal(driver.get_cookies(), [], "Selenium delete_all_cookies clears store")
    driver.get(f"{fixture}/webdriver/basic")
    record(results, "selenium_cookie_management")


def _run_selenium_wait_expected_conditions_smoke(
    driver: webdriver.Remote,
    fixture: str,
    results: list[dict[str, Any]],
) -> None:
    driver.get(f"{fixture}/webdriver/wait")
    wait = WebDriverWait(driver, 5, poll_frequency=0.05)
    stale_target = driver.find_element(By.ID, "remove-me")
    hide_target = driver.find_element(By.ID, "hide-me")
    check_target = driver.find_element(By.ID, "checky")

    late = wait.until(EC.presence_of_element_located((By.ID, "late")))
    assert_equal(late.text, "late ready", "Selenium WebDriverWait presence text")
    visible = wait.until(EC.visibility_of_element_located((By.ID, "late")))
    assert_equal(visible.text, "late ready", "Selenium WebDriverWait visibility text")
    assert_true(
        wait.until(EC.text_to_be_present_in_element((By.ID, "status"), "ready")),
        "Selenium expected condition text_to_be_present_in_element",
    )

    clickable = wait.until(EC.element_to_be_clickable((By.ID, "wait-clicker")))
    assert_equal(clickable.get_attribute("disabled"), None, "Selenium expected condition clickable enabled")
    assert_true(
        wait.until(EC.invisibility_of_element(hide_target)) is not False,
        "Selenium expected condition invisibility_of_element",
    )
    assert_true(
        wait.until(EC.invisibility_of_element_located((By.ID, "hide-located"))) is not False,
        "Selenium expected condition invisibility_of_element_located",
    )
    driver.execute_script("window.__removeWaitTarget()")
    assert_true(wait.until(EC.staleness_of(stale_target)), "Selenium expected condition staleness_of")
    assert_true(
        wait.until(EC.element_to_be_selected(check_target)),
        "Selenium expected condition element_to_be_selected",
    )
    assert_true(
        wait.until(EC.element_located_to_be_selected((By.ID, "checky"))),
        "Selenium expected condition element_located_to_be_selected",
    )
    assert_true(
        wait.until(EC.element_selection_state_to_be(check_target, True)),
        "Selenium expected condition element_selection_state_to_be",
    )
    assert_true(
        wait.until(EC.element_located_selection_state_to_be((By.ID, "checky"), True)),
        "Selenium expected condition element_located_selection_state_to_be",
    )
    assert_true(
        wait.until(EC.text_to_be_present_in_element_value((By.ID, "value-target"), "Expected")),
        "Selenium expected condition text_to_be_present_in_element_value",
    )
    assert_true(
        wait.until(EC.element_attribute_to_include((By.ID, "value-target"), "data-ready")),
        "Selenium expected condition element_attribute_to_include",
    )

    assert_true(
        wait.until(EC.frame_to_be_available_and_switch_to_it((By.ID, "delayed-frame"))),
        "Selenium expected condition frame_to_be_available_and_switch_to_it",
    )
    assert_equal(
        driver.find_element(By.ID, "inside-frame").text,
        "frame ready",
        "Selenium expected condition switched frame text",
    )
    driver.switch_to.default_content()

    driver.execute_script("setTimeout(() => alert('wait alert'), 0);")
    alert = wait.until(EC.alert_is_present())
    assert_equal(alert.text, "wait alert", "Selenium expected condition alert_is_present text")
    alert.accept()

    driver.get(f"{fixture}/webdriver/basic")
    record(results, "selenium_wait_expected_conditions")


def _run_selenium_implicit_wait_smoke(
    driver: webdriver.Remote,
    fixture: str,
    results: list[dict[str, Any]],
) -> None:
    # Reduced from Selenium Python implicit_waits_tests.py.
    page_url = f"{fixture}/webdriver/implicit-wait"
    try:
        driver.get(page_url)
        driver.implicitly_wait(2)
        driver.find_element(By.ID, "adder").click()
        box = driver.find_element(By.ID, "box0")
        assert_equal(box.text, "box 0", "Selenium implicit wait finds delayed single element")

        driver.get(page_url)
        driver.implicitly_wait(2)
        driver.find_element(By.ID, "adder").click()
        boxes = driver.find_elements(By.CLASS_NAME, "redbox")
        assert_true(
            len(boxes) >= 1,
            "Selenium implicit wait waits until find_elements has at least one match",
        )

        driver.get(page_url)
        driver.implicitly_wait(0.2)
        _assert_raises(
            NoSuchElementException,
            lambda: driver.find_element(By.ID, "box0"),
            "Selenium implicit wait still fails for missing single element",
        )
        assert_equal(
            driver.find_elements(By.CLASS_NAME, "redbox"),
            [],
            "Selenium implicit wait returns empty list for missing elements",
        )

        driver.get(page_url)
        driver.implicitly_wait(2)
        driver.implicitly_wait(0)
        driver.execute_script("window.__implicitAddDelayMs = 10000;")
        driver.find_element(By.ID, "adder").click()
        disabled_started = time.monotonic()
        _assert_raises(
            NoSuchElementException,
            lambda: driver.find_element(By.ID, "box0"),
            "Selenium disabled implicit wait returns after first single-element lookup",
        )
        assert_true(
            time.monotonic() - disabled_started < 1,
            "Selenium disabled implicit wait single-element lookup returns promptly",
        )
        disabled_started = time.monotonic()
        assert_equal(
            driver.find_elements(By.CLASS_NAME, "redbox"),
            [],
            "Selenium disabled implicit wait returns after first multi-element lookup",
        )
        assert_true(
            time.monotonic() - disabled_started < 1,
            "Selenium disabled implicit wait multi-element lookup returns promptly",
        )
    finally:
        driver.implicitly_wait(0)

    driver.get(f"{fixture}/webdriver/basic")
    record(results, "selenium_implicit_wait")


def _run_selenium_stale_element_smoke(
    driver: webdriver.Remote,
    fixture: str,
    results: list[dict[str, Any]],
) -> None:
    # Reduced from Selenium Python stale_reference_tests.py.
    driver.get(f"{fixture}/webdriver/basic")
    old_link = driver.find_element(By.ID, "relative-link")
    old_main = driver.find_element(By.ID, "main")
    driver.get(f"{fixture}/webdriver/form")
    _assert_raises(
        StaleElementReferenceException,
        lambda: old_link.click(),
        "Selenium stale element click after navigation",
    )
    _assert_raises(
        StaleElementReferenceException,
        lambda: old_main.get_attribute("class"),
        "Selenium stale element attribute after navigation",
    )
    _assert_raises(
        StaleElementReferenceException,
        lambda: old_main.rect,
        "Selenium stale element rect after navigation",
    )

    driver.get(f"{fixture}/webdriver/basic")
    removed = driver.find_element(By.ID, "main")
    driver.execute_script("arguments[0].remove()", removed)
    _assert_raises(
        StaleElementReferenceException,
        lambda: removed.text,
        "Selenium stale element text after DOM removal",
    )

    driver.get(f"{fixture}/webdriver/basic")
    record(results, "selenium_stale_element_references")


def _run_selenium_document_open_replacement_stale_element_smoke(
    driver: webdriver.Remote,
    fixture: str,
    results: list[dict[str, Any]],
) -> None:
    driver.get(f"{fixture}/webdriver/basic")
    driver.execute_script(
        "document.open(); document.write(arguments[0]); document.close(); return true;",
        "<!doctype html>"
        "<input id='snapshot-target' data-phase='old' value='old-value'>"
        "<p id='snapshot-marker'>old marker</p>",
    )
    old_element = driver.find_element(By.ID, "snapshot-target")
    old_element_id = old_element.id
    assert_equal(old_element.tag_name, "input", "Selenium old replacement tag name")
    assert_equal(
        old_element.get_attribute("data-phase"),
        "old",
        "Selenium old replacement data-phase",
    )
    assert_equal(
        old_element.get_attribute("value"),
        "old-value",
        "Selenium old replacement value",
    )

    driver.execute_script(
        "document.open(); document.write(arguments[0]); document.close(); return true;",
        "<!doctype html>"
        "<textarea id='snapshot-target' data-phase='new'>new-value</textarea>"
        "<p id='snapshot-marker'>new marker</p>",
    )
    _assert_raises(
        StaleElementReferenceException,
        lambda: old_element.tag_name,
        "Selenium old replacement element after document.open",
    )
    new_element = driver.find_element(By.ID, "snapshot-target")
    assert_true(
        new_element.id != old_element_id,
        "Selenium document.open replacement should allocate a new WebElement id",
    )
    assert_equal(new_element.tag_name, "textarea", "Selenium new replacement tag name")
    assert_equal(
        new_element.get_attribute("data-phase"),
        "new",
        "Selenium new replacement data-phase",
    )
    assert_equal(
        new_element.get_attribute("value"),
        "new-value",
        "Selenium new replacement value",
    )
    assert_true("new marker" in driver.page_source, "Selenium page source should include new replacement DOM")
    assert_true("old marker" not in driver.page_source, "Selenium page source should not retain old replacement DOM")
    record(
        results,
        "selenium_document_open_replacement_stale_element",
        {"oldElementId": old_element_id, "newElementId": new_element.id},
    )


def _run_selenium_nested_frame_switching_smoke(
    driver: webdriver.Remote,
    fixture: str,
    results: list[dict[str, Any]],
) -> None:
    # Reduced from Selenium Python frame_switching_tests.py.
    driver.get(f"{fixture}/webdriver/nested-frames")
    assert_equal(driver.find_element(By.ID, "top-main").text, "top ready", "Selenium frame top context")

    _assert_raises(
        NoSuchFrameException,
        lambda: driver.switch_to.frame(driver.find_element(By.ID, "top-main")),
        "Selenium frame rejects non-frame WebElement",
    )
    _assert_raises(
        NoSuchFrameException,
        lambda: driver.switch_to.frame(9),
        "Selenium frame rejects missing index",
    )

    driver.switch_to.frame("outerByName")
    assert_equal(driver.find_element(By.ID, "outer-main").text, "outer ready", "Selenium frame by name")
    _assert_raises(
        NoSuchFrameException,
        lambda: driver.switch_to.frame("siblingByName"),
        "Selenium frame lookup is relative to current frame",
    )
    driver.switch_to.frame("innerById")
    assert_equal(driver.find_element(By.ID, "inner-main").text, "inner ready", "Selenium nested frame by id")
    driver.switch_to.parent_frame()
    assert_equal(driver.find_element(By.ID, "outer-main").text, "outer ready", "Selenium parent_frame to outer")
    driver.switch_to.parent_frame()
    assert_equal(driver.find_element(By.ID, "top-main").text, "top ready", "Selenium parent_frame to top")

    driver.switch_to.frame("siblingById")
    assert_equal(driver.find_element(By.ID, "sibling-main").text, "sibling ready", "Selenium sibling frame by id")
    driver.switch_to.default_content()

    outer = driver.find_element(By.ID, "outerById")
    driver.switch_to.frame(outer)
    inner = driver.find_element(By.NAME, "innerByName")
    driver.switch_to.frame(inner)
    assert_equal(driver.find_element(By.ID, "inner-main").text, "inner ready", "Selenium frame by WebElement chain")
    driver.switch_to.default_content()
    assert_equal(driver.title, "WebDriver Smoke Nested Frames", "Selenium default_content restores top title")

    driver.get(f"{fixture}/webdriver/basic")
    record(results, "selenium_nested_frame_switching")


def _run_selenium_deleted_frame_recovery_smoke(
    driver: webdriver.Remote,
    fixture: str,
    results: list[dict[str, Any]],
) -> None:
    driver.get(f"{fixture}/webdriver/deleting-frame")
    wait = WebDriverWait(driver, 5, poll_frequency=0.05)

    wait.until(EC.frame_to_be_available_and_switch_to_it(0))
    _assert_deleting_frame_selected(driver, wait, "index")
    _delete_selected_frame_and_restore_top(driver, wait, "index")

    wait.until(EC.frame_to_be_available_and_switch_to_it("iframe1"))
    _assert_deleting_frame_selected(driver, wait, "string")
    _delete_selected_frame_and_restore_top(driver, wait, "string")

    iframe = driver.find_element(By.ID, "iframe1")
    wait.until(EC.frame_to_be_available_and_switch_to_it(iframe))
    _assert_deleting_frame_selected(driver, wait, "webelement")
    _delete_selected_frame_and_restore_top(driver, wait, "webelement")

    driver.get(f"{fixture}/webdriver/basic")
    record(results, "selenium_deleted_frame_recovery")


def _assert_deleting_frame_selected(
    driver: webdriver.Remote,
    wait: WebDriverWait,
    label: str,
) -> None:
    # Selenium's frame expected condition only waits until switch_to.frame()
    # succeeds; the child Document can still be loading at that point.
    success = wait.until(EC.presence_of_element_located((By.ID, "success")))
    assert_equal(
        success.text,
        "success",
        f"Selenium deleted frame {label} switched frame text",
    )


def _delete_selected_frame_and_restore_top(
    driver: webdriver.Remote,
    wait: WebDriverWait,
    label: str,
) -> None:
    driver.execute_script("window.frameElement?.remove();")
    driver.switch_to.default_content()
    wait.until_not(EC.presence_of_element_located((By.ID, "iframe1")))
    assert_equal(
        driver.find_element(By.ID, "top-marker").text,
        "top ready",
        f"Selenium deleted frame {label} default_content recovery",
    )
    driver.find_element(By.ID, "addBackFrame").click()
    wait.until(EC.presence_of_element_located((By.ID, "iframe1")))


def _run_selenium_form_file_actions_smoke(
    driver: webdriver.Remote,
    results: list[dict[str, Any]],
) -> None:
    check = driver.find_element(By.ID, "check")
    assert_equal(check.is_selected(), False, "Selenium checkbox initial selected state")
    check.click()
    assert_equal(check.is_selected(), True, "Selenium checkbox click selected state")

    Select(driver.find_element(By.ID, "select")).select_by_value("two")
    assert_equal(
        driver.execute_script("return document.getElementById('select').value;"),
        "two",
        "Selenium select option value",
    )
    assert_true(
        "select:change:two" in driver.execute_script("return window.__selectEvents.join(',');"),
        "Selenium select click dispatches change",
    )
    _assert_selenium_select_class(driver)

    clicker = driver.find_element(By.ID, "clicker")
    clicker.click()
    assert_equal(
        driver.find_element(By.ID, "click-output").text,
        "clicked",
        "Selenium element click side effect",
    )

    field = driver.find_element(By.ID, "field")
    field.clear()
    driver.execute_script("arguments[0].focus()", field)
    ActionChains(driver).send_keys("action").perform()
    assert_equal(field.get_property("value"), "action", "Selenium W3C actions key input")
    field.clear()
    driver.execute_script("arguments[0].focus()", field)
    ActionChains(driver).send_keys("fo").key_down(Keys.SHIFT).send_keys("ob").key_up(Keys.SHIFT).send_keys("ar").perform()
    assert_equal(
        field.get_property("value"),
        "foOBar",
        "Selenium W3C actions key input with modifiers",
    )

    with TemporaryDirectory(prefix="moli-webdriver-selenium-upload-") as tempdir:
        first = Path(tempdir) / "first.txt"
        second = Path(tempdir) / "second.txt"
        first.write_text("alpha", encoding="utf-8")
        second.write_text("bravo!", encoding="utf-8")
        upload = driver.find_element(By.ID, "upload-multiple")
        upload.send_keys(f"{first}\n{second}")
        file_summary = driver.execute_script(
            "const input = document.getElementById('upload-multiple');"
            "return [input.files.length, Array.from(input.files).map(file => file.name).join('|'),"
            "Array.from(input.files).map(file => file.size).join('|'), input.value, window.__fileEvents.join(',')].join('||');"
        )
        assert_equal(
            file_summary,
            f"2||{first.name}|{second.name}||5|6||C:\\fakepath\\{first.name}||upload-multiple:input:2,upload-multiple:change:2",
            "Selenium file input FileList",
        )
        file_text = driver.execute_async_script(
            """
            const done = arguments[arguments.length - 1];
            const input = document.getElementById('upload-multiple');
            Promise.all(Array.from(input.files).map(file => new Promise(resolve => {
              const reader = new FileReader();
              reader.onload = () => resolve(file.name + ':' + reader.result);
              reader.onerror = () => resolve(file.name + ':error');
              reader.readAsText(file);
            }))).then(values => done(values.join('|')));
            """
        )
        assert_equal(
            file_text,
            f"{first.name}:alpha|{second.name}:bravo!",
            "Selenium file input FileReader text",
        )

    record(results, "selenium_form_file_actions")


def _assert_selenium_select_class(driver: webdriver.Remote) -> None:
    # Reduced from Selenium Python select_class_tests.py.
    single_values = ["One", "Two", "Four", "Still learning how to count, apparently"]
    single_values_by_value = [value.lower() for value in single_values]
    single = Select(driver.find_element(By.NAME, "selectomatic"))
    assert_equal([option.text for option in single.options], single_values, "Selenium Select options")
    assert_equal(single.first_selected_option.text, "One", "Selenium Select first selected option")
    assert_equal(_selected_option_texts(single), ["One"], "Selenium Select selected options")
    for index, text in enumerate(single_values):
        single.select_by_index(index)
        assert_equal(single.first_selected_option.text, text, f"Selenium Select select_by_index {index}")
    for value, text in zip(single_values_by_value, single_values):
        single.select_by_value(value)
        assert_equal(single.first_selected_option.text, text, f"Selenium Select select_by_value {value}")
    for text in single_values:
        single.select_by_visible_text(text)
        assert_equal(single.first_selected_option.text, text, f"Selenium Select select_by_visible_text {text}")

    with_spaces = Select(driver.find_element(By.NAME, "select_with_spaces"))
    with_spaces.select_by_visible_text("Still learning how to count, apparently")
    assert_equal(
        with_spaces.first_selected_option.text,
        "Still learning how to count, apparently",
        "Selenium Select visible text normalizes spaces",
    )

    _assert_raises(
        NotImplementedError,
        lambda: single.deselect_all(),
        "Selenium Select deselect_all rejects single select",
    )
    _assert_raises(
        NotImplementedError,
        lambda: single.deselect_by_index(0),
        "Selenium Select deselect_by_index rejects single select",
    )
    _assert_raises(
        NotImplementedError,
        lambda: single.deselect_by_value("one"),
        "Selenium Select deselect_by_value rejects single select",
    )
    _assert_raises(
        NotImplementedError,
        lambda: single.deselect_by_visible_text("One"),
        "Selenium Select deselect_by_visible_text rejects single select",
    )

    multi_values = ["Eggs", "Ham", "Sausages", "Onion gravy"]
    multi_values_by_value = [value.lower() for value in multi_values]
    multi = Select(driver.find_element(By.NAME, "multi"))
    assert_true(multi.is_multiple, "Selenium Select recognizes multi-select")
    assert_equal(
        _selected_option_texts(multi),
        ["Eggs", "Sausages"],
        "Selenium Select initial multi selected options",
    )
    multi.deselect_all()
    assert_equal(_selected_option_texts(multi), [], "Selenium Select deselect_all multi")
    for index, text in enumerate(multi_values):
        multi.select_by_index(index)
        assert_equal(
            _selected_option_texts(multi),
            multi_values[: index + 1],
            f"Selenium Select multi select_by_index {index}",
        )
    multi.deselect_by_index(1)
    multi.deselect_by_index(3)
    assert_equal(
        _selected_option_texts(multi),
        ["Eggs", "Sausages"],
        "Selenium Select multi deselect_by_index",
    )
    multi.deselect_all()
    for value in multi_values_by_value:
        multi.select_by_value(value)
    multi.deselect_by_value("ham")
    multi.deselect_by_value("onion gravy")
    assert_equal(
        _selected_option_texts(multi),
        ["Eggs", "Sausages"],
        "Selenium Select multi deselect_by_value",
    )
    multi.deselect_all()
    for text in multi_values:
        multi.select_by_visible_text(text)
    multi.deselect_by_visible_text("Ham")
    multi.deselect_by_visible_text("Onion gravy")
    assert_equal(
        _selected_option_texts(multi),
        ["Eggs", "Sausages"],
        "Selenium Select multi deselect_by_visible_text",
    )

    empty_multi = Select(driver.find_element(By.NAME, "select_empty_multiple"))
    assert_equal(empty_multi.all_selected_options, [], "Selenium Select empty multi starts unselected")
    empty_multi.deselect_all()
    assert_equal(empty_multi.all_selected_options, [], "Selenium Select empty multi deselect_all")
    _assert_raises(
        NoSuchElementException,
        lambda: empty_multi.deselect_by_index(10),
        "Selenium Select missing deselect_by_index",
    )
    _assert_raises(
        NoSuchElementException,
        lambda: empty_multi.deselect_by_value("not there"),
        "Selenium Select missing deselect_by_value",
    )
    _assert_raises(
        NoSuchElementException,
        lambda: empty_multi.deselect_by_visible_text("not there"),
        "Selenium Select missing deselect_by_visible_text",
    )

    _assert_raises(
        NotImplementedError,
        lambda: Select(driver.find_element(By.NAME, "single_disabled")).select_by_visible_text("Disabled"),
        "Selenium Select disabled single option",
    )
    _assert_raises(
        NotImplementedError,
        lambda: Select(driver.find_element(By.NAME, "multi_disabled")).select_by_value("disabled"),
        "Selenium Select disabled multi option",
    )
    _assert_raises(
        NoSuchElementException,
        lambda: Select(driver.find_element(By.ID, "invisible-multi-select")).select_by_visible_text("Apples"),
        "Selenium Select hidden option",
    )
    _assert_raises(
        UnexpectedTagNameException,
        lambda: Select(driver.find_element(By.ID, "main")),
        "Selenium Select rejects non-select element",
    )


def _selected_option_texts(select: Select) -> list[str]:
    return [option.text for option in select.all_selected_options]


def _assert_raises(
    exception_type: type[BaseException],
    action: Callable[[], object],
    message: str,
) -> None:
    try:
        action()
    except exception_type:
        return
    except Exception as error:
        raise AssertionError(f"{message}: expected {exception_type.__name__}, got {type(error).__name__}") from error
    raise AssertionError(f"{message}: expected {exception_type.__name__}")


def _assert_webdriver_error_contains(action: Callable[[], object], expected: str, message: str) -> None:
    try:
        action()
    except WebDriverException as error:
        assert_true(expected in str(error), f"{message}: {error}")
        return
    except Exception as error:
        raise AssertionError(f"{message}: expected WebDriverException, got {type(error).__name__}") from error
    raise AssertionError(f"{message}: expected WebDriverException")


def _run_selenium_form_submit_typing_smoke(
    driver: webdriver.Remote,
    fixture: str,
    results: list[dict[str, Any]],
) -> None:
    # Reduced from Selenium Python form_handling_tests.py, typing_tests.py,
    # and correct_event_firing_tests.py.
    form_url = f"{fixture}/webdriver/form"
    wait = WebDriverWait(driver, 5, poll_frequency=0.05)

    def submit_and_assert(locator: tuple[str, str], login_value: str, label: str) -> None:
        progress_name = f"selenium_form_submit_typing[{label}]"
        record_progress("selenium", progress_name, "load")
        driver.get(form_url)
        login = driver.find_element(By.ID, "login")
        login.clear()
        login.send_keys(login_value)
        record_progress("selenium", progress_name, "submit")
        driver.find_element(*locator).submit()
        record_progress("selenium", progress_name, "wait-title")
        wait.until(EC.title_is("WebDriver Submitted"))
        submitted = driver.find_element(By.ID, "submitted").text
        assert_true(
            f'"login":["{login_value}"]' in submitted,
            f"Selenium form submit {label} submitted login value",
        )
        record_progress("selenium", progress_name, "done")

    submit_and_assert((By.NAME, "login"), "input-submit", "input in form")
    submit_and_assert((By.ID, "checky"), "checkbox-submit", "checkbox in form")
    submit_and_assert((By.ID, "submit-paragraph"), "paragraph-submit", "element in form")
    submit_and_assert((By.ID, "submit"), "id-submit", "element with id submit")
    submit_and_assert((By.NAME, "submit"), "name-submit", "element with name submit")

    driver.get(form_url)
    driver.find_element(By.ID, "login").send_keys("click-submit")
    driver.find_element(By.ID, "submit").click()
    wait.until(EC.title_is("WebDriver Submitted"))
    assert_true(
        '"login":["click-submit"]' in driver.find_element(By.ID, "submitted").text,
        "Selenium submit input click navigates form",
    )

    driver.get(form_url)
    _assert_raises(
        WebDriverException,
        lambda: driver.find_element(By.NAME, "SearchableText").submit(),
        "Selenium submit outside form",
    )

    driver.get(form_url)
    checkbox = driver.find_element(By.ID, "checky")
    assert_equal(checkbox.is_selected(), False, "Selenium form checkbox initial state")
    checkbox.click()
    assert_equal(checkbox.is_selected(), True, "Selenium form checkbox click selected")
    assert_equal(driver.find_element(By.ID, "result").text, "checkbox:true", "Selenium checkbox change event")
    checkbox.click()
    assert_equal(checkbox.is_selected(), False, "Selenium form checkbox second click unselected")

    selected_radio = driver.find_element(By.ID, "cheese-and-peas")
    cheese = driver.find_element(By.ID, "cheese")
    peas = driver.find_element(By.ID, "peas")
    assert_equal(selected_radio.is_selected(), True, "Selenium radio initial selected")
    assert_equal(cheese.is_selected(), False, "Selenium radio sibling initial unselected")
    cheese.click()
    assert_equal(cheese.is_selected(), True, "Selenium radio click selected")
    assert_equal(peas.is_selected(), False, "Selenium radio sibling remains unselected")
    peas.click()
    assert_equal(cheese.is_selected(), False, "Selenium radio previous sibling unselected")
    assert_equal(peas.is_selected(), True, "Selenium radio sibling selected")

    working = driver.find_element(By.ID, "working")
    working.send_keys("Some")
    working.send_keys(" text")
    assert_equal(working.get_attribute("value"), "Some text", "Selenium form input append text")
    driver.execute_script("window.__formEvents = []; document.getElementById('event-log').textContent = '';")
    working.clear()
    assert_equal(working.get_attribute("value"), "", "Selenium form input clear")
    events = _form_event_log(driver)
    assert_true("working:input" in events, "Selenium clear fires input event")
    assert_true("working:change" in events, "Selenium clear fires change event")

    no_type = driver.find_element(By.ID, "no-type")
    no_type.send_keys("Should Say Cheese")
    assert_equal(no_type.get_attribute("value"), "Should Say Cheese", "Selenium input without type")

    textarea = driver.find_element(By.ID, "with-text")
    driver.execute_script("window.__formEvents = []; document.getElementById('event-log').textContent = '';")
    textarea.clear()
    textarea.send_keys("Brie and cheddar")
    assert_equal(textarea.get_attribute("value"), "Brie and cheddar", "Selenium textarea send keys")
    driver.execute_script("window.__formEvents = []; document.getElementById('event-log').textContent = '';")
    textarea.clear()
    assert_equal(textarea.get_attribute("value"), "", "Selenium textarea clear")
    events = _form_event_log(driver)
    assert_true("with-text:input" in events, "Selenium textarea clear fires input event")
    assert_true("with-text:change" in events, "Selenium textarea clear fires change event")

    key_reporter = driver.find_element(By.ID, "key-reporter")
    key_reporter.send_keys("abc def")
    assert_equal(key_reporter.get_attribute("value"), "abc def", "Selenium typing lowercase and space")
    assert_true("key-reporter:keydown:a:" in _form_event_log(driver), "Selenium typing keydown event")
    assert_true("key-reporter:keyup:a:" in _form_event_log(driver), "Selenium typing keyup event")

    key_reporter.clear()
    key_reporter.send_keys("Tet", Keys.ARROW_LEFT, "s")
    assert_equal(key_reporter.get_attribute("value"), "Test", "Selenium arrow key caret insertion")

    key_reporter.clear()
    key_reporter.send_keys(Keys.ARROW_LEFT)
    assert_equal(key_reporter.get_attribute("value"), "", "Selenium arrow key is not printable")

    driver.execute_script(
        "window.__formEvents = []; document.getElementById('event-log').textContent = '';"
    )
    for selenium_key, dom_key, key_code in [
        (Keys.ARROW_DOWN, "ArrowDown", 40),
        (Keys.ARROW_UP, "ArrowUp", 38),
        (Keys.ARROW_LEFT, "ArrowLeft", 37),
        (Keys.ARROW_RIGHT, "ArrowRight", 39),
    ]:
        key_reporter.send_keys(selenium_key)
        events = _form_event_log(driver)
        assert_true(
            f"key-reporter:keydown:{dom_key}:{key_code}" in events,
            f"Selenium {dom_key} keydown reports legacy keyCode",
        )
        assert_true(
            f"key-reporter:keyup:{dom_key}:{key_code}" in events,
            f"Selenium {dom_key} keyup reports legacy keyCode",
        )
    assert_equal(
        key_reporter.get_attribute("value"),
        "",
        "Selenium arrow keyCode reporting leaves no printable text",
    )

    key_reporter.clear()
    key_reporter.send_keys(
        f"a{Keys.ARROW_LEFT}b{Keys.ARROW_RIGHT}"
        f"{Keys.ARROW_UP}{Keys.ARROW_DOWN}{Keys.PAGE_UP}{Keys.PAGE_DOWN}1"
    )
    assert_equal(
        key_reporter.get_attribute("value"),
        "ba1",
        "Selenium page up/down keys are not printable",
    )

    key_reporter.clear()
    all_printable = "!\"#$%&'()*+,-./0123456789:<=>?@ ABCDEFGHIJKLMNOPQRSTUVWXYZ [\\]^_`abcdefghijklmnopqrstuvwxyz{|}~"
    key_reporter.send_keys(all_printable)
    assert_equal(key_reporter.get_attribute("value"), all_printable, "Selenium all printable send_keys")

    key_reporter.clear()
    key_reporter.send_keys(
        "abcd"
        + Keys.MULTIPLY
        + Keys.SUBTRACT
        + Keys.ADD
        + Keys.DECIMAL
        + Keys.SEPARATOR
        + Keys.NUMPAD0
        + Keys.NUMPAD9
        + Keys.ADD
        + Keys.SEMICOLON
        + Keys.EQUALS
        + Keys.DIVIDE
        + Keys.NUMPAD3
        + "abcd"
    )
    assert_equal(
        key_reporter.get_attribute("value"),
        "abcd*-+.,09+;=/3abcd",
        "Selenium numberpad private keys insert printable values",
    )
    key_reporter.clear()
    key_reporter.send_keys("FUNCTION", Keys.F2, "-KEYS", Keys.F2)
    key_reporter.send_keys(Keys.F2, "-TOO", Keys.F2)
    assert_equal(
        key_reporter.get_attribute("value"),
        "FUNCTION-KEYS-TOO",
        "Selenium function private keys are not printable",
    )

    key_reporter.clear()
    key_reporter.send_keys("abcd efgh")
    key_reporter.send_keys(Keys.SHIFT, Keys.ARROW_LEFT, Keys.ARROW_LEFT, Keys.ARROW_LEFT)
    key_reporter.send_keys(Keys.DELETE)
    assert_equal(
        key_reporter.get_attribute("value"),
        "abcd e",
        "Selenium send_keys Shift+Arrow selection delete",
    )

    driver.execute_script(
        """
        const input = arguments[0];
        input.value = "";
        input.addEventListener("keydown", event => {
          if (event.key === "l" || event.key === "o") {
            event.preventDefault();
          }
        }, { once: false });
        """,
        key_reporter,
    )
    key_reporter.send_keys("Hello World!")
    assert_equal(
        key_reporter.get_attribute("value"),
        "He Wrd!",
        "Selenium typing honors canceled keydown",
    )

    key_reporter.clear()
    key_reporter.send_keys("abcdefghi")
    assert_equal(key_reporter.get_attribute("value"), "abcdefghi", "Selenium typing before delete")
    key_reporter.send_keys(Keys.ARROW_LEFT, Keys.ARROW_LEFT, Keys.DELETE)
    assert_equal(key_reporter.get_attribute("value"), "abcdefgi", "Selenium delete key removes forward char")
    key_reporter.send_keys(Keys.ARROW_LEFT, Keys.ARROW_LEFT, Keys.BACKSPACE)
    assert_equal(key_reporter.get_attribute("value"), "abcdfgi", "Selenium backspace removes previous char")

    key_reporter.clear()
    key_reporter.send_keys(1234)
    assert_equal(key_reporter.get_attribute("value"), "1234", "Selenium integer send_keys")

    driver.execute_script(
        """
        const input = arguments[0];
        input.value = "some text";
        input.focus();
        input.setSelectionRange(input.value.length, input.value.length);
        input.addEventListener("keydown", event => {
          if ((event.ctrlKey || event.metaKey) && event.key === "a") {
            event.preventDefault();
          }
        }, { once: false });
        """,
        key_reporter,
    )
    (
        ActionChains(driver)
        .key_down(Keys.CONTROL)
        .send_keys("a")
        .key_up(Keys.CONTROL)
        .send_keys(Keys.BACKSPACE)
        .perform()
    )
    assert_equal(
        key_reporter.get_attribute("value"),
        "some tex",
        "Selenium key actions honor canceled select-all default",
    )

    driver.execute_script(
        """
        const input = arguments[0];
        input.value = "";
        input.focus();
        window.__seleniumRepeatEvents = [];
        input.addEventListener("keydown", event => {
          window.__seleniumRepeatEvents.push(`${event.key}:${event.repeat}`);
        }, { once: false });
        """,
        key_reporter,
    )
    (
        ActionChains(driver)
        .key_down("a")
        .key_down("a")
        .key_up("a")
        .key_down("a")
        .key_up("a")
        .perform()
    )
    assert_equal(
        driver.execute_script("return window.__seleniumRepeatEvents;"),
        ["a:false", "a:true", "a:false"],
        "Selenium key actions repeat property",
    )

    driver.execute_script("window.__formEvents = []; document.getElementById('event-log').textContent = '';")
    first = driver.find_element(By.ID, "event-one")
    second = driver.find_element(By.ID, "event-two")
    first.send_keys("foo")
    second.send_keys("bar")
    events = _form_event_log(driver)
    assert_true("event-one:focus" in events, "Selenium send_keys fires focus")
    assert_true("event-one:input" in events, "Selenium send_keys fires input")
    assert_true("event-one:change" in events, "Selenium send_keys commits text input change on blur")
    assert_true("event-one:blur" in events, "Selenium send_keys to another element fires blur")
    assert_true("event-two:focus" in events, "Selenium second send_keys fires focus")
    assert_true("event-two:input" in events, "Selenium second send_keys fires input")

    driver.get(f"{fixture}/webdriver/basic")
    record(results, "selenium_form_submit_typing")


def _form_event_log(driver: webdriver.Remote) -> str:
    return driver.find_element(By.ID, "event-log").text


def _run_selenium_pointer_actions_smoke(
    driver: webdriver.Remote,
    fixture: str,
    results: list[dict[str, Any]],
) -> None:
    driver.get(f"{fixture}/webdriver/actions")

    hover = driver.find_element(By.ID, "hover")
    _assert_webdriver_error_contains(
        lambda: ActionChains(driver).move_to_element(hover).perform(),
        "layout hit testing",
        "Selenium ActionChains hover coordinate unsupported error",
    )
    assert_equal(_action_event_log(driver), "", "Selenium ActionChains hover does not dispatch")

    double_click = driver.find_element(By.ID, "double-click")
    _assert_webdriver_error_contains(
        lambda: ActionChains(driver).double_click(double_click).perform(),
        "layout hit testing",
        "Selenium ActionChains double_click coordinate unsupported error",
    )
    assert_equal(
        double_click.get_attribute("value"),
        "",
        "Selenium ActionChains unsupported double_click value",
    )
    assert_equal(_action_event_log(driver), "", "Selenium ActionChains double_click does not dispatch")

    context_click = driver.find_element(By.ID, "context-click")
    _assert_webdriver_error_contains(
        lambda: ActionChains(driver).context_click(context_click).perform(),
        "layout hit testing",
        "Selenium ActionChains context_click coordinate unsupported error",
    )
    assert_equal(
        context_click.get_attribute("value"),
        "",
        "Selenium ActionChains unsupported context_click value",
    )
    assert_equal(_action_event_log(driver), "", "Selenium ActionChains context_click does not dispatch")

    source = driver.find_element(By.ID, "drag-source")
    target = driver.find_element(By.ID, "drop-target")
    _assert_webdriver_error_contains(
        lambda: ActionChains(driver).click_and_hold(source).move_to_element(target).release(target).perform(),
        "layout hit testing",
        "Selenium ActionChains drag/drop coordinate unsupported error",
    )
    assert_equal(
        target.find_element(By.TAG_NAME, "p").text,
        "Drop here",
        "Selenium ActionChains unsupported drag/drop target text",
    )
    assert_equal(
        target.get_attribute("data-drop"),
        None,
        "Selenium ActionChains unsupported drag/drop data transfer",
    )
    assert_equal(_action_event_log(driver), "", "Selenium ActionChains drag/drop does not dispatch")

    driver.get(f"{fixture}/webdriver/basic")
    record(results, "selenium_pointer_action_coordinate_boundary")


def _action_event_log(driver: webdriver.Remote) -> str:
    return driver.find_element(By.ID, "event-log").text


def _run_selenium_screenshot_print_unsupported_cookie_window_smoke(
    driver: webdriver.Remote,
    fixture: str,
    results: list[dict[str, Any]],
) -> None:
    driver.get(f"{fixture}/webdriver/basic")
    _assert_webdriver_error_contains(
        driver.get_screenshot_as_png,
        "Page.captureScreenshot is not supported",
        "Selenium screenshot unsupported error",
    )
    main = driver.find_element(By.ID, "main")
    _assert_webdriver_error_contains(
        lambda: main.screenshot_as_png,
        "Page.captureScreenshot is not supported",
        "Selenium element screenshot unsupported error",
    )

    _assert_webdriver_error_contains(
        driver.print_page,
        "Page.printToPDF is not supported",
        "Selenium print_page default unsupported error",
    )
    print_options = PrintOptions()
    print_options.page_ranges = ["1"]
    print_options.orientation = "landscape"
    print_options.width = 30
    assert_true("sessionId" not in print_options.to_dict(), "Selenium PrintOptions initial shape")
    _assert_webdriver_error_contains(
        lambda: driver.print_page(print_options),
        "Page.printToPDF is not supported",
        "Selenium print_page options unsupported error",
    )
    assert_true(
        "sessionId" not in print_options.to_dict(),
        "Selenium print_page should not mutate PrintOptions session id",
    )

    original_url = driver.current_url
    cookie_url = f"{fixture}/webdriver/cookie-echo"
    driver.get(cookie_url)
    driver.add_cookie({"name": "seleniumSmoke", "value": "cookie", "path": "/"})
    assert_equal(
        driver.get_cookie("seleniumSmoke")["value"],
        "cookie",
        "Selenium named cookie value",
    )
    assert_true(
        any(cookie.get("name") == "seleniumSmoke" for cookie in driver.get_cookies()),
        "Selenium cookie list",
    )
    driver.delete_cookie("seleniumSmoke")
    assert_equal(driver.get_cookie("seleniumSmoke"), None, "Selenium deleted cookie lookup")

    driver.back()
    assert_equal(driver.current_url, original_url, "Selenium back navigation URL")
    driver.forward()
    assert_equal(driver.current_url, cookie_url, "Selenium forward navigation URL")
    driver.back()
    driver.refresh()
    assert_equal(driver.current_url, original_url, "Selenium refresh keeps URL")

    original_handle = driver.current_window_handle
    driver.switch_to.new_window("tab")
    new_handle = driver.current_window_handle
    assert_true(new_handle != original_handle, "Selenium new window handle")
    assert_true(original_handle in driver.window_handles, "Selenium original window handle listed")
    driver.get(f"{fixture}/webdriver/basic")
    assert_equal(driver.title, "WebDriver Smoke Basic", "Selenium new window navigation title")
    driver.close()
    driver.switch_to.window(original_handle)
    assert_equal(driver.current_url, original_url, "Selenium switch back original window")

    rect = driver.get_window_rect()
    assert_true(rect["width"] > 0 and rect["height"] > 0, "Selenium window rect read")
    restored = driver.set_window_rect(0, 0, 800, 600)
    assert_equal(restored["width"], 800, "Selenium set window rect width")
    driver.maximize_window()
    maximized = driver.get_window_rect()
    assert_true(maximized["width"] >= 800 and maximized["height"] >= 600, "Selenium maximize window rect")
    driver.minimize_window()
    minimized = driver.get_window_rect()
    assert_equal(minimized, maximized, "Selenium minimize preserves headless rect")
    hidden_surface = driver.execute_script(
        "return JSON.stringify({ hasFocus: document.hasFocus(), hidden: document.hidden, visibilityState: document.visibilityState });"
    )
    assert_equal(
        hidden_surface,
        '{"hasFocus":false,"hidden":true,"visibilityState":"hidden"}',
        "Selenium minimized document surface",
    )
    driver.set_window_rect(rect["x"], rect["y"], rect["width"], rect["height"])

    record(results, "selenium_screenshot_print_unsupported_cookie_window")


def _run_selenium_popup_window_workflow_smoke(
    driver: webdriver.Remote,
    fixture: str,
    results: list[dict[str, Any]],
) -> None:
    source_url = f"{fixture}/webdriver/popup-page"
    driver.get(source_url)
    wait = WebDriverWait(driver, 5, poll_frequency=0.05)
    original_handle = driver.current_window_handle

    anchor_url = f"{fixture}/webdriver/popup-target?kind=anchor"
    anchor_handle = _open_new_window_from_current_handles(
        driver,
        lambda: driver.find_element(By.ID, "anchor-popup").click(),
        "Selenium anchor target popup handle",
    )
    _switch_and_assert_popup(driver, wait, anchor_handle, anchor_url, "popup anchor", "anchor target popup")
    driver.close()
    driver.switch_to.window(original_handle)
    assert_equal(driver.current_url, source_url, "Selenium anchor popup leaves opener URL")

    script_url = f"{fixture}/webdriver/popup-target?kind=script"
    script_handle = _open_new_window_from_current_handles(
        driver,
        lambda: driver.find_element(By.ID, "script-popup").click(),
        "Selenium script window.open popup handle",
    )
    _switch_and_assert_popup(driver, wait, script_handle, script_url, "popup script", "script window.open popup")
    driver.close()
    driver.switch_to.window(original_handle)

    named_first_url = f"{fixture}/webdriver/popup-target?kind=named-first"
    named_handle = _open_new_window_from_current_handles(
        driver,
        lambda: driver.find_element(By.ID, "named-link").click(),
        "Selenium named target popup handle",
    )
    _switch_and_assert_popup(driver, wait, named_handle, named_first_url, "popup named-first", "named target popup")
    driver.switch_to.window(original_handle)

    named_second_url = f"{fixture}/webdriver/popup-target?kind=named-second"
    handle_count = len(driver.window_handles)
    driver.find_element(By.ID, "named-reuse").click()
    driver.switch_to.window(named_handle)
    wait.until(lambda active_driver: active_driver.current_url == named_second_url)
    assert_equal(
        len(driver.window_handles),
        handle_count,
        "Selenium named target reuse should not create a second popup",
    )
    named_text = wait.until(lambda active_driver: _element_text_or_false(active_driver, By.ID, "popup-main"))
    assert_equal(named_text, "popup named-second", "Selenium named target reused popup text")
    driver.close()
    driver.switch_to.window(original_handle)

    self_url = f"{fixture}/webdriver/popup-target?kind=self"
    driver.find_element(By.ID, "self-open").click()
    wait.until(lambda active_driver: active_driver.current_url == self_url)
    self_text = wait.until(lambda active_driver: _element_text_or_false(active_driver, By.ID, "popup-main"))
    assert_equal(self_text, "popup self", "Selenium window.open _self navigation text")

    driver.get(f"{fixture}/webdriver/basic")
    record(results, "selenium_popup_window_workflow")


def _open_new_window_from_current_handles(
    driver: webdriver.Remote,
    trigger: Callable[[], None],
    label: str,
) -> str:
    previous_handles = set(driver.window_handles)
    trigger()
    handle = WebDriverWait(driver, 5, poll_frequency=0.05).until(
        lambda active_driver: _first_new_window_handle(active_driver, previous_handles)
    )
    assert_true(handle not in previous_handles, label)
    return handle


def _first_new_window_handle(driver: webdriver.Remote, previous_handles: set[str]) -> str | bool:
    for handle in driver.window_handles:
        if handle not in previous_handles:
            return handle
    return False


def _switch_and_assert_popup(
    driver: webdriver.Remote,
    wait: WebDriverWait,
    handle: str,
    expected_url: str,
    expected_text: str,
    label: str,
) -> None:
    driver.switch_to.window(handle)
    wait.until(lambda active_driver: active_driver.current_url == expected_url)
    assert_equal(driver.current_url, expected_url, f"Selenium {label} URL")
    actual_text = wait.until(lambda active_driver: _element_text_or_false(active_driver, By.ID, "popup-main"))
    assert_equal(actual_text, expected_text, f"Selenium {label} text")


def _element_text_or_false(driver: webdriver.Remote, by: str, value: str) -> str | bool:
    try:
        return driver.find_element(by, value).text
    except WebDriverException:
        return False


def _run_selenium_shadow_root_and_dialog_smoke(
    driver: webdriver.Remote,
    results: list[dict[str, Any]],
) -> None:
    # Ported in reduced form from Selenium Python web_components_tests.py.
    host = driver.find_element(By.ID, "host")
    shadow_root = host.shadow_root
    assert_true(isinstance(shadow_root, ShadowRoot), "Selenium element.shadow_root object")

    shadow_text = shadow_root.find_element(By.CSS_SELECTOR, "#shadow-text")
    assert_true(isinstance(shadow_text, WebElement), "Selenium shadow root find_element WebElement")
    assert_equal(shadow_text.text, "shadow ready", "Selenium shadow root text")
    shadow_items = shadow_root.find_elements(By.CSS_SELECTOR, ".shadow-item")
    assert_equal(len(shadow_items), 2, "Selenium shadow root find_elements length")
    assert_true(
        all(isinstance(element, WebElement) for element in shadow_items),
        "Selenium shadow root find_elements WebElements",
    )

    execute_shadow_root = driver.execute_script("return arguments[0].shadowRoot", host)
    assert_equal(execute_shadow_root, shadow_root, "Selenium execute_script shadow root equality")

    try:
        driver.find_element(By.ID, "main").shadow_root
        raise AssertionError("non-host element should not have a Selenium shadow root")
    except NoSuchShadowRootException:
        pass

    wait = WebDriverWait(driver, 2)

    opened = driver.execute_script(
        "setTimeout(() => alert('selenium smoke alert'), 0); return 'opened';"
    )
    assert_equal(opened, "opened", "Selenium alert setup script result")
    alert = wait.until(EC.alert_is_present())
    assert_equal(alert.text, "selenium smoke alert", "Selenium alert text")
    alert.accept()

    opened = driver.execute_script(
        "setTimeout(() => confirm('selenium smoke confirm'), 0); return 'confirm opened';"
    )
    assert_equal(opened, "confirm opened", "Selenium confirm setup script result")
    confirm = wait.until(EC.alert_is_present())
    assert_equal(confirm.text, "selenium smoke confirm", "Selenium confirm text")
    confirm.dismiss()

    opened = driver.execute_script(
        "setTimeout(() => prompt('selenium smoke prompt', 'default'), 0); return 'prompt opened';"
    )
    assert_equal(opened, "prompt opened", "Selenium prompt setup script result")
    prompt = wait.until(EC.alert_is_present())
    assert_equal(prompt.text, "selenium smoke prompt", "Selenium prompt text")
    prompt.send_keys("typed prompt")
    prompt.accept()

    # Prove the session still accepts commands after the alert route handled the prompt.
    assert_true(driver.find_element(By.ID, "main").is_displayed(), "Selenium post-alert command")
    record(results, "selenium_shadow_root_dialog", {"shadowText": shadow_text.text})
