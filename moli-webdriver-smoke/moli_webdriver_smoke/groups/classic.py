from __future__ import annotations

import asyncio
from pathlib import Path
from tempfile import TemporaryDirectory
from typing import Any, Awaitable, Callable

from ..assertions import assert_equal, assert_true, record
from ..client import (
    CLASSIC_ELEMENT_REFERENCE_KEY,
    ClassicClient,
    WebDriverHttpError,
    classic_element_id,
    classic_shadow_root_id,
    classic_value,
)
from ..scenarios import record_failure


ClassicScenario = Callable[[ClassicClient, str, str, list[dict[str, Any]]], Awaitable[None]]
CAPTURE_SCREENSHOT_UNSUPPORTED_MESSAGE = (
    "Page.captureScreenshot is not supported: renderer screenshots are not implemented."
)


async def _wait_for_alert_text(
    client: ClassicClient,
    session_id: str,
    expected: str,
    *,
    timeout: float = 2.0,
) -> None:
    loop = asyncio.get_running_loop()
    deadline = loop.time() + timeout
    while True:
        try:
            actual = classic_value(client.get(f"/session/{session_id}/alert/text"))
        except WebDriverHttpError as error:
            value = error.response.body.get("value", {})
            if (
                error.response.status != 404
                or not isinstance(value, dict)
                or value.get("error") != "no such alert"
            ):
                raise
            if loop.time() >= deadline:
                raise AssertionError(f"timed out waiting for alert {expected!r}") from error
            await asyncio.sleep(0.01)
            continue
        assert_equal(actual, expected, "Classic asynchronously opened alert text")
        return


async def _wait_for_current_url(
    client: ClassicClient,
    session_id: str,
    expected: str,
    *,
    timeout: float = 5.0,
) -> None:
    loop = asyncio.get_running_loop()
    deadline = loop.time() + timeout
    observed: list[str] = []
    while True:
        actual = classic_value(client.get(f"/session/{session_id}/url"))
        if actual == expected:
            return
        if not observed or observed[-1] != actual:
            observed.append(actual)
        if loop.time() >= deadline:
            raise AssertionError(
                f"timed out waiting for Classic URL {expected!r}; observed {observed!r}"
            )
        await asyncio.sleep(0.01)


async def run_classic_group(
    endpoint: str,
    fixture: str,
    results: list[dict[str, Any]],
    continue_on_failure: bool = False,
) -> None:
    if continue_on_failure:
        for name, scenario in _classic_scenarios():
            try:
                await _run_classic_isolated_scenario(endpoint, fixture, results, scenario)
            except Exception as error:
                record_failure(results, "classic", name, error)
        return

    client = ClassicClient(endpoint)
    status = client.get("/status")
    assert_equal(classic_value(status)["ready"], True, "WebDriver Classic status ready")

    session = client.post("/session", {"capabilities": {"alwaysMatch": {}}})
    session_id = classic_value(session)["sessionId"]
    assert_true(isinstance(session_id, str) and session_id, "Classic session id should be non-empty")
    record(results, "classic_session_new", {"sessionId": session_id})

    try:
        for _name, scenario in _classic_scenarios():
            await scenario(client, fixture, session_id, results)
    finally:
        client.delete(f"/session/{session_id}")
        record(results, "classic_session_delete")


def _classic_scenarios() -> tuple[tuple[str, ClassicScenario], ...]:
    return (
        ("classic_navigation_element_script", _run_navigation_element_script_smoke),
        ("classic_document_open_replacement_stale_element", _run_document_open_replacement_stale_element_smoke),
        ("classic_input_navigation_replacement", _run_input_navigation_replacement_smoke),
        ("classic_clear_form", _run_clear_form_smoke),
        ("classic_file_upload", _run_file_upload_smoke),
        ("classic_alert_prompt", _run_alert_smoke),
        ("classic_window_prompt", _run_window_prompt_smoke),
        ("classic_shadow_root", _run_shadow_root_smoke),
        ("classic_shared_worker", _run_shared_worker_smoke),
        ("classic_cookies", _run_cookie_smoke),
        ("classic_window_state", _run_window_state_smoke),
    )


async def _run_classic_isolated_scenario(
    endpoint: str,
    fixture: str,
    results: list[dict[str, Any]],
    scenario: ClassicScenario,
) -> None:
    client = ClassicClient(endpoint)
    status = client.get("/status")
    assert_equal(classic_value(status)["ready"], True, "WebDriver Classic status ready")

    session = client.post("/session", {"capabilities": {"alwaysMatch": {}}})
    session_id = classic_value(session)["sessionId"]
    assert_true(isinstance(session_id, str) and session_id, "Classic session id should be non-empty")
    record(results, "classic_session_new", {"sessionId": session_id})

    scenario_error: BaseException | None = None
    try:
        await scenario(client, fixture, session_id, results)
    except Exception as error:
        scenario_error = error
    finally:
        try:
            client.delete(f"/session/{session_id}")
            record(results, "classic_session_delete")
        except Exception as error:
            if scenario_error is None:
                scenario_error = error
    if scenario_error is not None:
        raise scenario_error


async def _run_navigation_element_script_smoke(
    client: ClassicClient,
    fixture: str,
    session_id: str,
    results: list[dict[str, Any]],
) -> None:
    page_url = f"{fixture}/webdriver/basic"
    assert_equal(
        client.post(f"/session/{session_id}/url", {"url": page_url}),
        {"value": None},
        "Classic navigate response",
    )
    assert_equal(classic_value(client.get(f"/session/{session_id}/url")), page_url, "Classic current URL")
    assert_equal(
        classic_value(client.get(f"/session/{session_id}/title")),
        "WebDriver Smoke Basic",
        "Classic title",
    )
    assert_true(
        "Basic Ready" in classic_value(client.get(f"/session/{session_id}/source")),
        "Classic page source should contain fixture text",
    )

    main = client.post(
        f"/session/{session_id}/element",
        {"using": "css selector", "value": "#main"},
    )
    main_id = classic_element_id(main)
    xpath_main = client.post(
        f"/session/{session_id}/element",
        {"using": "xpath", "value": "//main[@id='main' and contains(@class, 'cheese')]"},
    )
    assert_equal(
        classic_element_id(xpath_main),
        main_id,
        "Classic CSS and XPath locators should return the same element identity",
    )
    assert_equal(
        classic_value(client.get(f"/session/{session_id}/element/{main_id}/text")),
        "Basic Ready",
        "Classic element text",
    )
    assert_equal(
        classic_value(client.get(f"/session/{session_id}/element/{main_id}/name")),
        "main",
        "Classic element tag name",
    )
    assert_equal(
        classic_value(client.get(f"/session/{session_id}/element/{main_id}/property/classList")),
        ["no", "cheese"],
        "Classic DOMTokenList property value",
    )
    assert_equal(
        classic_value(client.get(f"/session/{session_id}/element/{main_id}/displayed")),
        True,
        "Classic displayed state",
    )
    assert_equal(
        classic_value(client.get(f"/session/{session_id}/element/{main_id}/enabled")),
        True,
        "Classic enabled state",
    )
    rect = classic_value(client.get(f"/session/{session_id}/element/{main_id}/rect"))
    for key in ("x", "y", "width", "height"):
        assert_true(isinstance(rect.get(key), (int, float)), f"Classic element rect {key} should be numeric")

    labelled = client.post(
        f"/session/{session_id}/element",
        {"using": "css selector", "value": "#labelled"},
    )
    labelled_id = classic_element_id(labelled)
    assert_equal(
        classic_value(client.get(f"/session/{session_id}/element/{labelled_id}/computedlabel")),
        "Smoke Label",
        "Classic computed label",
    )
    buttons = classic_value(
        client.post(
            f"/session/{session_id}/execute/sync",
            {"script": "return document.querySelectorAll('button');", "args": []},
        )
    )
    assert_true(
        isinstance(buttons, list) and len(buttons) >= 2,
        "Classic execute script NodeList result",
    )
    first_button_id = (
        buttons[0].get(CLASSIC_ELEMENT_REFERENCE_KEY)
        if isinstance(buttons[0], dict)
        else None
    )
    assert_true(
        isinstance(first_button_id, str) and first_button_id,
        "Classic NodeList item should be WebElement",
    )
    assert_equal(
        classic_value(
            client.get(f"/session/{session_id}/element/{first_button_id}/equals/{labelled_id}")
        ),
        True,
        "Classic NodeList item should match labelled button",
    )

    article = client.post(
        f"/session/{session_id}/element",
        {"using": "css selector", "value": "#article-role"},
    )
    article_id = classic_element_id(article)
    assert_equal(
        classic_value(client.get(f"/session/{session_id}/element/{article_id}/computedrole")),
        "article",
        "Classic computed role",
    )

    echoed = client.post(
        f"/session/{session_id}/execute/sync",
        {
            "script": "return arguments[0].textContent + ':' + document.title;",
            "args": [classic_value(main)],
        },
    )
    assert_equal(
        classic_value(echoed),
        "Basic Ready:WebDriver Smoke Basic",
        "Classic execute script with element argument",
    )

    field = client.post(
        f"/session/{session_id}/element",
        {"using": "css selector", "value": "#field"},
    )
    field_id = classic_element_id(field)
    assert_equal(
        client.post(f"/session/{session_id}/element/{field_id}/value", {"text": "typed"}),
        {"value": None},
        "Classic send keys response",
    )
    assert_equal(
        classic_value(client.get(f"/session/{session_id}/element/{field_id}/property/value")),
        "typed",
        "Classic send keys value",
    )
    assert_equal(
        classic_value(client.get(f"/session/{session_id}/element/{field_id}/attribute/checked")),
        None,
        "Classic missing boolean attribute",
    )

    multiple = client.post(
        f"/session/{session_id}/element",
        {"using": "css selector", "value": "#upload-multiple"},
    )
    multiple_id = classic_element_id(multiple)
    assert_equal(
        classic_value(client.get(f"/session/{session_id}/element/{multiple_id}/attribute/multiple")),
        "true",
        "Classic boolean attribute value",
    )

    hidden = client.post(
        f"/session/{session_id}/element",
        {"using": "css selector", "value": "#hidden-attribute"},
    )
    hidden_id = classic_element_id(hidden)
    assert_equal(
        classic_value(client.get(f"/session/{session_id}/element/{hidden_id}/attribute/hidden")),
        "true",
        "Classic global boolean attribute value",
    )

    relative_link = client.post(
        f"/session/{session_id}/element",
        {"using": "css selector", "value": "#relative-link"},
    )
    relative_link_id = classic_element_id(relative_link)
    assert_equal(
        classic_value(client.get(f"/session/{session_id}/element/{relative_link_id}/attribute/href")),
        "/webdriver/basic",
        "Classic href content attribute value",
    )
    assert_equal(
        classic_value(client.get(f"/session/{session_id}/element/{relative_link_id}/property/href")),
        page_url,
        "Classic href property absolute value",
    )

    clicker = client.post(
        f"/session/{session_id}/element",
        {"using": "css selector", "value": "#clicker"},
    )
    clicker_id = classic_element_id(clicker)
    assert_equal(
        client.post(f"/session/{session_id}/element/{clicker_id}/click"),
        {"value": None},
        "Classic element click response",
    )
    clicked = client.post(
        f"/session/{session_id}/execute/sync",
        {"script": "return document.body.dataset.clicked + ':' + document.querySelector('#click-output').textContent;", "args": []},
    )
    assert_equal(classic_value(clicked), "yes:clicked", "Classic click side effect")

    screenshot = client.get(f"/session/{session_id}/screenshot", expected_status=405)
    _assert_classic_unsupported(
        screenshot,
        CAPTURE_SCREENSHOT_UNSUPPORTED_MESSAGE,
        "Classic screenshot unsupported response",
    )
    record(results, "classic_navigation_element_script", {"elementId": main_id})
    record(results, "classic_screenshot_unsupported")


async def _run_document_open_replacement_stale_element_smoke(
    client: ClassicClient,
    fixture: str,
    session_id: str,
    results: list[dict[str, Any]],
) -> None:
    client.post(f"/session/{session_id}/url", {"url": f"{fixture}/webdriver/basic"})
    assert_equal(
        classic_value(
            client.post(
                f"/session/{session_id}/execute/sync",
                {
                    "script": (
                        "document.open();"
                        "document.write(arguments[0]);"
                        "document.close();"
                        "return true;"
                    ),
                    "args": [
                        "<!doctype html>"
                        "<input id='snapshot-target' data-phase='old' value='old-value'>"
                        "<p id='snapshot-marker'>old marker</p>",
                    ],
                },
            )
        ),
        True,
        "Classic old document.open replacement result",
    )
    old_element = client.post(
        f"/session/{session_id}/element",
        {"using": "css selector", "value": "#snapshot-target"},
    )
    old_element_id = classic_element_id(old_element)
    assert_equal(
        classic_value(client.get(f"/session/{session_id}/element/{old_element_id}/name")),
        "input",
        "Classic old replacement tag name",
    )
    assert_equal(
        classic_value(client.get(f"/session/{session_id}/element/{old_element_id}/attribute/data-phase")),
        "old",
        "Classic old replacement data-phase",
    )

    assert_equal(
        classic_value(
            client.post(
                f"/session/{session_id}/execute/sync",
                {
                    "script": (
                        "document.open();"
                        "document.write(arguments[0]);"
                        "document.close();"
                        "return true;"
                    ),
                    "args": [
                        "<!doctype html>"
                        "<textarea id='snapshot-target' data-phase='new'>new-value</textarea>"
                        "<p id='snapshot-marker'>new marker</p>",
                    ],
                },
            )
        ),
        True,
        "Classic new document.open replacement result",
    )
    _assert_classic_stale_element(
        lambda: client.get(f"/session/{session_id}/element/{old_element_id}/name"),
        "Classic old replacement element tag after document.open",
    )

    new_element = client.post(
        f"/session/{session_id}/element",
        {"using": "css selector", "value": "#snapshot-target"},
    )
    new_element_id = classic_element_id(new_element)
    assert_true(
        new_element_id != old_element_id,
        "Classic document.open replacement should allocate a new WebElement id",
    )
    assert_equal(
        classic_value(client.get(f"/session/{session_id}/element/{new_element_id}/name")),
        "textarea",
        "Classic new replacement tag name",
    )
    assert_equal(
        classic_value(client.get(f"/session/{session_id}/element/{new_element_id}/attribute/data-phase")),
        "new",
        "Classic new replacement data-phase",
    )
    assert_equal(
        classic_value(client.get(f"/session/{session_id}/element/{new_element_id}/property/value")),
        "new-value",
        "Classic new replacement value",
    )
    page_source = classic_value(client.get(f"/session/{session_id}/source"))
    assert_true("new marker" in page_source, "Classic page source should include new replacement DOM")
    assert_true("old marker" not in page_source, "Classic page source should not retain old replacement DOM")
    record(
        results,
        "classic_document_open_replacement_stale_element",
        {"oldElementId": old_element_id, "newElementId": new_element_id},
    )


async def _run_input_navigation_replacement_smoke(
    client: ClassicClient,
    fixture: str,
    session_id: str,
    results: list[dict[str, Any]],
) -> None:
    page_url = f"{fixture}/webdriver/input-navigation"
    destination = f"{fixture}/webdriver/input-navigation-complete"
    assert_equal(
        client.post(f"/session/{session_id}/url", {"url": page_url}),
        {"value": None},
        "Classic input-navigation setup",
    )

    field = client.post(
        f"/session/{session_id}/element",
        {"using": "css selector", "value": "#navigation-field"},
    )
    field_id = classic_element_id(field)
    assert_equal(
        client.post(f"/session/{session_id}/element/{field_id}/click"),
        {"value": None},
        "Classic input-navigation focus",
    )

    actions = client.post(
        f"/session/{session_id}/actions",
        {
            "actions": [
                {
                    "type": "key",
                    "id": "navigation-keyboard",
                    "actions": [{"type": "keyDown", "value": "\ue007"}],
                }
            ]
        },
    )
    assert_equal(
        actions,
        {"value": None},
        "Classic input action responds across Page replacement",
    )
    await _wait_for_current_url(client, session_id, destination)
    _assert_classic_stale_element(
        lambda: client.get(f"/session/{session_id}/element/{field_id}/name"),
        "Classic input-navigation source element after Page replacement",
    )
    assert_true(
        "input navigation complete" in classic_value(client.get(f"/session/{session_id}/source")),
        "Classic input action replacement Page remains usable",
    )
    assert_equal(
        client.delete(f"/session/{session_id}/actions"),
        {"value": None},
        "Classic input action release after Page replacement",
    )
    record(results, "classic_input_navigation_replacement", {"url": destination})


def _assert_classic_stale_element(action: Callable[[], Any], label: str) -> None:
    try:
        action()
    except WebDriverHttpError as error:
        assert_equal(error.response.status, 404, f"{label} HTTP status")
        assert_equal(
            error.response.body["value"]["error"],
            "stale element reference",
            f"{label} error",
        )
        return
    raise AssertionError(f"{label} should fail with stale element reference")


async def _run_clear_form_smoke(
    client: ClassicClient,
    fixture: str,
    session_id: str,
    results: list[dict[str, Any]],
) -> None:
    client.post(f"/session/{session_id}/url", {"url": f"{fixture}/webdriver/basic"})

    field = client.post(
        f"/session/{session_id}/element",
        {"using": "css selector", "value": "#field"},
    )
    field_id = classic_element_id(field)
    assert_equal(
        client.post(f"/session/{session_id}/element/{field_id}/value", {"text": "to clear"}),
        {"value": None},
        "Classic clear smoke setup send keys",
    )
    client.post(
        f"/session/{session_id}/execute/sync",
        {"script": "window.__clearEvents = []; return null;", "args": []},
    )
    assert_equal(
        client.post(f"/session/{session_id}/element/{field_id}/clear"),
        {"value": None},
        "Classic clear writable input response",
    )
    assert_equal(
        classic_value(client.get(f"/session/{session_id}/element/{field_id}/property/value")),
        "",
        "Classic clear writable input value",
    )
    clear_events = client.post(
        f"/session/{session_id}/execute/sync",
        {"script": "return window.__clearEvents.join('|');", "args": []},
    )
    assert_equal(
        classic_value(clear_events),
        "field:input:|field:change:",
        "Classic clear dispatches input/change events",
    )

    legend = client.post(
        f"/session/{session_id}/element",
        {"using": "css selector", "value": "#disabled-fieldset-legend"},
    )
    legend_id = classic_element_id(legend)
    assert_equal(
        client.post(f"/session/{session_id}/element/{legend_id}/clear"),
        {"value": None},
        "Classic clear disabled fieldset first legend response",
    )
    assert_equal(
        classic_value(client.get(f"/session/{session_id}/element/{legend_id}/property/value")),
        "",
        "Classic clear disabled fieldset first legend value",
    )

    for selector in ("#disabled-fieldset-child", "#hidden-clear"):
        element = client.post(
            f"/session/{session_id}/element",
            {"using": "css selector", "value": selector},
        )
        element_id = classic_element_id(element)
        try:
            client.post(f"/session/{session_id}/element/{element_id}/clear")
            raise AssertionError(f"{selector} clear should fail")
        except WebDriverHttpError as error:
            assert_equal(error.response.status, 400, f"Classic {selector} clear HTTP status")
            assert_equal(
                error.response.body["value"]["error"],
                "invalid element state",
                f"Classic {selector} clear error",
            )
    record(results, "classic_clear_form")


async def _run_file_upload_smoke(
    client: ClassicClient,
    fixture: str,
    session_id: str,
    results: list[dict[str, Any]],
) -> None:
    client.post(f"/session/{session_id}/url", {"url": f"{fixture}/webdriver/basic"})
    with TemporaryDirectory(prefix="moli-webdriver-classic-upload-") as tempdir:
        first = Path(tempdir) / "first.txt"
        second = Path(tempdir) / "second.txt"
        first.write_text("alpha", encoding="utf-8")
        second.write_text("bravo!", encoding="utf-8")

        multiple = client.post(
            f"/session/{session_id}/element",
            {"using": "css selector", "value": "#upload-multiple"},
        )
        multiple_id = classic_element_id(multiple)
        assert_equal(
            client.post(
                f"/session/{session_id}/element/{multiple_id}/value",
                {"text": f"{first}\n{second}"},
            ),
            {"value": None},
            "Classic file input send keys response",
        )
        summary = classic_value(
            client.post(
                f"/session/{session_id}/execute/sync",
                {
                    "script": "const input = document.getElementById('upload-multiple'); return [input.files.length, Array.from(input.files).map(file => file.name).join('|'), Array.from(input.files).map(file => file.size).join('|'), input.value, window.__fileEvents.join(',')].join('||');",
                    "args": [],
                },
            )
        )
        assert_equal(
            summary,
            f"2||{first.name}|{second.name}||5|6||C:\\fakepath\\{first.name}||upload-multiple:input:2,upload-multiple:change:2",
            "Classic file input FileList",
        )
        file_text = classic_value(
            client.post(
                f"/session/{session_id}/execute/async",
                {
                    "script": (
                        "const done = arguments[arguments.length - 1];"
                        "const input = document.getElementById('upload-multiple');"
                        "Promise.all(Array.from(input.files).map(file => new Promise(resolve => {"
                        "  const reader = new FileReader();"
                        "  reader.onload = () => resolve(file.name + ':' + reader.result);"
                        "  reader.onerror = () => resolve(file.name + ':error');"
                        "  reader.readAsText(file);"
                        "}))).then(values => done(values.join('|')));"
                    ),
                    "args": [],
                },
            )
        )
        assert_equal(
            file_text,
            f"{first.name}:alpha|{second.name}:bravo!",
            "Classic file input FileReader text",
        )

        single = client.post(
            f"/session/{session_id}/element",
            {"using": "css selector", "value": "#upload-single"},
        )
        single_id = classic_element_id(single)
        try:
            client.post(
                f"/session/{session_id}/element/{single_id}/value",
                {"text": f"{first}\n{second}"},
            )
            raise AssertionError("non-multiple file input should reject two files")
        except WebDriverHttpError as error:
            assert_equal(error.response.status, 400, "Classic non-multiple upload HTTP status")
            assert_equal(error.response.body["value"]["error"], "invalid argument", "Classic non-multiple upload error")
    record(results, "classic_file_upload")


async def _run_alert_smoke(
    client: ClassicClient,
    fixture: str,
    session_id: str,
    results: list[dict[str, Any]],
) -> None:
    client.post(f"/session/{session_id}/url", {"url": f"{fixture}/webdriver/basic"})
    try:
        client.get(f"/session/{session_id}/alert/text")
        raise AssertionError("missing alert text should fail")
    except WebDriverHttpError as error:
        assert_equal(error.response.status, 404, "Classic missing alert HTTP status")
        assert_equal(error.response.body["value"]["error"], "no such alert", "Classic missing alert error")

    opened = client.post(
        f"/session/{session_id}/execute/sync",
        {
            "script": "setTimeout(() => alert('classic smoke alert'), 0); return 'opened';",
            "args": [],
        },
    )
    assert_equal(classic_value(opened), "opened", "Classic execute alert result")
    await _wait_for_alert_text(client, session_id, "classic smoke alert")
    assert_equal(
        client.post(f"/session/{session_id}/alert/accept"),
        {"value": None},
        "Classic alert accept",
    )

    client.post(
        f"/session/{session_id}/execute/sync",
        {
            "script": "setTimeout(() => prompt('Prompt?', 'default'), 0); return 'prompt opened';",
            "args": [],
        },
    )
    await _wait_for_alert_text(client, session_id, "Prompt?")
    assert_equal(
        client.post(f"/session/{session_id}/alert/text", {"text": "cheese"}),
        {"value": None},
        "Classic prompt send text",
    )
    assert_equal(
        client.post(f"/session/{session_id}/alert/accept"),
        {"value": None},
        "Classic prompt accept",
    )

    client.post(
        f"/session/{session_id}/execute/sync",
        {
            "script": "setTimeout(() => alert('classic unhandled prompt'), 0); return 'opened';",
            "args": [],
        },
    )
    await _wait_for_alert_text(client, session_id, "classic unhandled prompt")
    try:
        client.get(f"/session/{session_id}/title")
        raise AssertionError("default unhandledPromptBehavior should notify on an open alert")
    except WebDriverHttpError as error:
        assert_equal(error.response.status, 500, "Classic unhandled prompt HTTP status")
        assert_equal(
            error.response.body["value"]["error"],
            "unexpected alert open",
            "Classic unhandled prompt error",
        )
        assert_equal(
            error.response.body["value"]["data"],
            {"text": "classic unhandled prompt"},
            "Classic unhandled prompt data",
        )
    try:
        client.get(f"/session/{session_id}/alert/text")
        raise AssertionError("default unhandledPromptBehavior should dismiss the alert")
    except WebDriverHttpError as error:
        assert_equal(error.response.status, 404, "Classic auto-dismissed alert HTTP status")
        assert_equal(
            error.response.body["value"]["error"],
            "no such alert",
            "Classic auto-dismissed alert error",
        )
    record(results, "classic_alert_prompt")


async def _run_window_prompt_smoke(
    client: ClassicClient,
    fixture: str,
    session_id: str,
    results: list[dict[str, Any]],
) -> None:
    client.post(f"/session/{session_id}/url", {"url": f"{fixture}/webdriver/basic"})
    original_handle = classic_value(client.get(f"/session/{session_id}/window"))
    created = classic_value(client.post(f"/session/{session_id}/window/new", {"type": "tab"}))
    new_handle = created["handle"]
    assert_true(
        isinstance(new_handle, str) and new_handle,
        "Classic new window handle should be non-empty",
    )
    assert_true(
        new_handle != original_handle,
        "Classic new window handle should differ from original",
    )

    opened = client.post(
        f"/session/{session_id}/execute/sync",
        {
            "script": "setTimeout(() => alert('window prompt smoke'), 0); return 'opened';",
            "args": [],
        },
    )
    assert_equal(classic_value(opened), "opened", "Classic window prompt setup")
    await _wait_for_alert_text(client, session_id, "window prompt smoke")
    assert_equal(
        classic_value(client.get(f"/session/{session_id}/window")),
        original_handle,
        "Classic get window should not handle the open alert",
    )
    handles = classic_value(client.get(f"/session/{session_id}/window/handles"))
    assert_true(
        original_handle in handles and new_handle in handles,
        "Classic get window handles with alert",
    )

    assert_equal(
        client.post(f"/session/{session_id}/window", {"handle": new_handle}),
        {"value": None},
        "Classic switch away from prompted window",
    )
    try:
        client.get(f"/session/{session_id}/alert/text")
        raise AssertionError("switched window should not see original alert")
    except WebDriverHttpError as error:
        assert_equal(error.response.status, 404, "Classic switched alert HTTP status")
        assert_equal(
            error.response.body["value"]["error"],
            "no such alert",
            "Classic switched alert error",
        )

    assert_equal(
        client.post(f"/session/{session_id}/window", {"handle": original_handle}),
        {"value": None},
        "Classic switch back to prompted window",
    )
    assert_equal(
        classic_value(client.get(f"/session/{session_id}/alert/text")),
        "window prompt smoke",
        "Classic original window alert text",
    )
    assert_equal(
        client.post(f"/session/{session_id}/alert/accept"),
        {"value": None},
        "Classic original alert accept",
    )

    assert_equal(
        client.post(f"/session/{session_id}/window", {"handle": new_handle}),
        {"value": None},
        "Classic switch to close smoke tab",
    )
    remaining = classic_value(client.delete(f"/session/{session_id}/window"))
    assert_true(
        original_handle in remaining and new_handle not in remaining,
        "Classic close smoke tab handles",
    )
    assert_equal(
        client.post(f"/session/{session_id}/window", {"handle": original_handle}),
        {"value": None},
        "Classic switch back after closing smoke tab",
    )
    record(results, "classic_window_prompt")


async def _run_shadow_root_smoke(
    client: ClassicClient,
    fixture: str,
    session_id: str,
    results: list[dict[str, Any]],
) -> None:
    client.post(f"/session/{session_id}/url", {"url": f"{fixture}/webdriver/basic"})
    host = client.post(
        f"/session/{session_id}/element",
        {"using": "css selector", "value": "#host"},
    )
    host_id = classic_element_id(host)
    shadow = client.get(f"/session/{session_id}/element/{host_id}/shadow")
    shadow_id = classic_shadow_root_id(shadow)
    inside = client.post(
        f"/session/{session_id}/shadow/{shadow_id}/element",
        {"using": "css selector", "value": "#shadow-text"},
    )
    inside_id = classic_element_id(inside)
    assert_equal(
        classic_value(client.get(f"/session/{session_id}/element/{inside_id}/text")),
        "shadow ready",
        "Classic shadow child text",
    )
    all_items = client.post(
        f"/session/{session_id}/shadow/{shadow_id}/elements",
        {"using": "class name", "value": "shadow-item"},
    )
    assert_equal(len(classic_value(all_items)), 2, "Classic shadow scoped elements length")
    record(results, "classic_shadow_root", {"shadowRootId": shadow_id})


async def _run_cookie_smoke(
    client: ClassicClient,
    fixture: str,
    session_id: str,
    results: list[dict[str, Any]],
) -> None:
    client.post(f"/session/{session_id}/url", {"url": f"{fixture}/webdriver/cookie-echo"})
    client.post(
        f"/session/{session_id}/cookie",
        {"cookie": {"name": "webdriverSmoke", "value": "classic", "path": "/"}},
    )
    named_cookie = classic_value(client.get(f"/session/{session_id}/cookie/webdriverSmoke"))
    assert_equal(named_cookie["value"], "classic", "Classic named cookie value")
    all_cookies = classic_value(client.get(f"/session/{session_id}/cookie"))
    assert_true(any(cookie.get("name") == "webdriverSmoke" for cookie in all_cookies), "Classic cookie list")
    client.delete(f"/session/{session_id}/cookie/webdriverSmoke")
    try:
        client.get(f"/session/{session_id}/cookie/webdriverSmoke")
        raise AssertionError("deleted cookie lookup should fail")
    except WebDriverHttpError as error:
        assert_equal(error.response.status, 404, "Classic deleted cookie HTTP status")
        assert_equal(error.response.body["value"]["error"], "no such cookie", "Classic deleted cookie error")
    record(results, "classic_cookies")


async def _run_shared_worker_smoke(
    client: ClassicClient,
    fixture: str,
    session_id: str,
    results: list[dict[str, Any]],
) -> None:
    page_url = f"{fixture}/webdriver/shared-worker"
    initial_handles = classic_value(client.get(f"/session/{session_id}/window/handles"))
    client.post(f"/session/{session_id}/url", {"url": page_url})
    assert_equal(classic_value(client.get(f"/session/{session_id}/title")), "WebDriver Smoke SharedWorker", "Classic shared worker title")
    assert_equal(
        classic_value(
            client.post(
                f"/session/{session_id}/execute/async",
                {
                    "script": (
                        "const done = arguments[arguments.length - 1];"
                        "globalThis.__webdriverSharedWorkerProbe('classic')"
                        ".then(done, error => done({ kind: 'error', error: String(error) }));"
                    ),
                    "args": [],
                },
            )
        ),
        {
            "kind": "probe-result",
            "echoed": "classic",
            "name": "webdriver-shared-worker-smoke",
            "pathname": "/webdriver/shared-worker.js",
            "selfEqualsGlobal": True,
            "isSharedWorker": True,
            "connectionId": 1,
            "connectionCount": 1,
        },
        "Classic shared worker probe result",
    )
    handles = classic_value(client.get(f"/session/{session_id}/window/handles"))
    assert_equal(handles, initial_handles, "Classic shared worker should not add a window handle")
    record(results, "classic_shared_worker")


async def _run_window_state_smoke(
    client: ClassicClient,
    fixture: str,
    session_id: str,
    results: list[dict[str, Any]],
) -> None:
    client.post(f"/session/{session_id}/url", {"url": "data:text/html,<title>window state</title><main>window</main>"})
    maximized = classic_value(client.post(f"/session/{session_id}/window/maximize"))
    assert_true(maximized["width"] >= 800 and maximized["height"] >= 600, "Classic maximize should set a headless rect")

    minimized = classic_value(client.post(f"/session/{session_id}/window/minimize"))
    assert_equal(minimized, maximized, "Classic minimize preserves current headless rect")
    minimized_surface = classic_value(
        client.post(
            f"/session/{session_id}/execute/sync",
            {
                "script": "return JSON.stringify({ hasFocus: document.hasFocus(), hidden: document.hidden, visibilityState: document.visibilityState });",
                "args": [],
            },
        )
    )
    assert_equal(
        minimized_surface,
        '{"hasFocus":false,"hidden":true,"visibilityState":"hidden"}',
        "Classic minimized document surface",
    )

    restored = classic_value(
        client.post(
            f"/session/{session_id}/window/rect",
            {"x": 0, "y": 0, "width": 800, "height": 600},
        )
    )
    assert_equal(restored["width"], 800, "Classic restored window width")
    visible_surface = classic_value(
        client.post(
            f"/session/{session_id}/execute/sync",
            {
                "script": "return JSON.stringify({ hasFocus: document.hasFocus(), hidden: document.hidden, visibilityState: document.visibilityState });",
                "args": [],
            },
        )
    )
    assert_equal(
        visible_surface,
        '{"hasFocus":true,"hidden":false,"visibilityState":"visible"}',
        "Classic restored document surface",
    )
    record(results, "classic_window_state")


def _assert_classic_unsupported(response: dict[str, Any], expected_message: str, label: str) -> None:
    value = classic_value(response)
    assert_true(isinstance(value, dict), f"{label} value should be an object")
    assert_equal(value.get("error"), "unsupported operation", f"{label} error")
    assert_equal(value.get("message"), expected_message, f"{label} message")
